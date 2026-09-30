//! The scan: what is running right now that Quiet Mode could park but the
//! saved targets do not cover, and folding accepted finds back into them.

use std::time::{SystemTime, UNIX_EPOCH};

use cq_core::{Profile, Recommendation, RecommendationKind, Risk, Settings, Snapshot, recommend};
use serde::Serialize;

use crate::engine::Engine;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub recommendations: Vec<Recommendation>,
    /// Seconds since the epoch.
    pub scanned_at: u64,
    /// Whether the platform could tell which programs own a window; without
    /// it only recognised software is listed.
    pub activity_known: bool,
    pub cached_bytes: u64,
}

/// The finds that can be parked without asking: low risk and not already in
/// the profile. The memory purge is never one of them: it is opt-in, so it is
/// suggested on the Scan tab and switched on only by the user.
pub fn low_risk_additions(recommendations: &[Recommendation]) -> Vec<Recommendation> {
    recommendations
        .iter()
        .filter(|item| {
            item.risk == Risk::Low
                && !item.already_targeted
                && item.kind != RecommendationKind::MemoryPurge
        })
        .cloned()
        .collect()
}

impl Engine {
    /// Take a fresh look at the machine against the saved profile.
    pub fn scan(&self) -> Result<ScanReport, AppError> {
        let settings = self.settings();
        let names = recommend::service_names_to_query(&settings.profile, self.platform().os());
        let snapshot = self.platform().snapshot(&names)?;
        self.report(&settings.profile, &snapshot)
    }

    /// Recommendations for a snapshot already taken (the quiet run reuses its
    /// own snapshot rather than sampling twice).
    pub(crate) fn report(
        &self,
        profile: &Profile,
        snapshot: &Snapshot,
    ) -> Result<ScanReport, AppError> {
        let stats = self.platform().stats()?;
        let activity = self.platform().activity();
        let recommendations = recommend::recommend(
            profile,
            snapshot,
            &stats,
            &activity,
            cq_platform::current_pid(),
            self.platform().os(),
            &self.platform().capabilities(),
        );
        Ok(ScanReport {
            recommendations,
            scanned_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            activity_known: activity.known,
            cached_bytes: stats.memory_available.saturating_sub(stats.memory_free),
        })
    }

    /// Add accepted finds to the saved targets. Names arrive from the page and
    /// are validated by the settings layer before anything is written.
    pub fn apply_recommendations(
        &self,
        accepted: Vec<Recommendation>,
    ) -> Result<Settings, AppError> {
        let mut settings = self.settings();
        settings.profile = recommend::apply(&settings.profile, &accepted);
        self.save_settings(settings.clone())?;
        Ok(settings)
    }
}

#[cfg(all(test, feature = "fake-platform"))]
mod tests {
    use std::sync::Arc;

    use cq_core::{RecommendationKind, ServiceTarget};

    use super::*;

    fn engine(dir: &std::path::Path) -> Engine {
        Engine::new(Arc::new(cq_platform::fake::Fake::new()), dir.to_path_buf())
    }

    #[test]
    fn the_fake_machine_yields_known_hogs_a_heuristic_find_and_system_savings() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let mut settings = engine.settings();
        settings.profile.processes.clear();
        settings.profile.services = vec![ServiceTarget {
            name: "SysMain".into(),
            enabled: true,
        }];
        settings.profile.power = cq_core::PowerPolicy::Leave;
        settings.profile.purge_memory = false;
        engine.save_settings(settings).unwrap();

        let report = engine.scan().unwrap();
        assert!(report.activity_known);
        let names: Vec<(&str, Risk, bool)> = report
            .recommendations
            .iter()
            .map(|r| (r.name.as_str(), r.risk, r.already_targeted))
            .collect();
        assert!(
            names.contains(&("GoogleUpdate.exe", Risk::Low, false)),
            "{names:?}"
        );
        assert!(names.contains(&("SysMain", Risk::Low, true)), "{names:?}");
        assert!(names.contains(&("WSearch", Risk::Low, false)), "{names:?}");
        assert!(
            names.contains(&("Power plan: Balanced", Risk::Low, false)),
            "{names:?}"
        );
        assert!(
            names.contains(&("Cached memory", Risk::Low, false)),
            "{names:?}"
        );
        assert!(
            names.contains(&("render-farm.exe", Risk::Medium, false)),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|(n, ..)| *n == "game.exe"),
            "the foreground game is never suggested"
        );
        assert!(!names.iter().any(|(n, ..)| *n == "explorer.exe"));

        let low = low_risk_additions(&report.recommendations);
        assert!(
            low.iter()
                .all(|r| r.risk == Risk::Low && !r.already_targeted)
        );
        let applied = engine.apply_recommendations(low.clone()).unwrap();
        assert_eq!(applied.profile.power, cq_core::PowerPolicy::Performance);
        assert!(
            !low.iter()
                .any(|r| r.kind == RecommendationKind::MemoryPurge),
            "the purge is suggested, never applied unasked"
        );
        assert!(!applied.profile.purge_memory);
        let cache = report
            .recommendations
            .iter()
            .find(|r| r.kind == RecommendationKind::MemoryPurge)
            .unwrap();
        let accepted = engine.apply_recommendations(vec![cache.clone()]).unwrap();
        assert!(accepted.profile.purge_memory, "accepting it switches it on");
        assert!(
            applied
                .profile
                .processes
                .iter()
                .any(|t| t.name == "GoogleUpdate.exe")
        );
        assert!(applied.profile.services.iter().any(|t| t.name == "WSearch"));
        assert!(
            !applied
                .profile
                .processes
                .iter()
                .any(|t| t.name == "render-farm.exe")
        );

        // Applied once, the same finds are now "already targeted".
        let again = engine.scan().unwrap();
        for item in again.recommendations {
            if matches!(item.kind, RecommendationKind::Process { .. }) && item.risk == Risk::Low {
                assert!(item.already_targeted, "{}", item.name);
            }
        }
    }
}
