//! Switching Quiet Mode on: plan from the machine as it is, write the journal,
//! then carry out each step.

use cq_core::journal::Summary;
use cq_core::watch::{Ending, running};
use cq_core::{CoreError, Journal, Plan, Recommendation, Settings, Step};

use super::preview::Planned;
use super::{Engine, LogLine, RunReport, now};
use crate::error::AppError;

impl Engine {
    /// Switch Quiet Mode on. `ending` is how the run ends by itself, if it
    /// does; a `Trigger` one also means nobody pressed the button, which
    /// changes what the run is willing to do (see `Plan::unattended`).
    /// `profile` names the profile to run this once. Without one, a run that
    /// a program started uses the profile chosen for that program, and any
    /// other the active profile.
    pub fn go_quiet(
        &self,
        progress: &dyn Fn(LogLine),
        ending: Option<Ending>,
        profile: Option<&str>,
    ) -> Result<Summary, AppError> {
        let _guard = self.begin()?;
        let chosen = profile
            .map(str::to_string)
            .or_else(|| self.profile_for_trigger(ending.as_ref()));
        let mut settings = self.runnable_settings(chosen.as_deref())?;
        self.lock().run_report = None;
        protect_what_it_waits_for(&mut settings, ending.as_ref());
        // Planned from the machine as it is now, never from an earlier preview.
        let Planned {
            snapshot,
            mut plan,
            added,
        } = self.plan_now(&settings)?;
        check_it_is_running(ending.as_ref(), &snapshot.processes)?;

        let mut log = Vec::new();
        // With one profile there is nothing to choose between, so nothing to say.
        let using = (!settings.other_profiles.is_empty()).then(|| used_profile(&settings));
        let started_by = match &ending {
            Some(Ending::Trigger { program }) => {
                plan.unattended();
                Some(started_because(program, &plan))
            }
            _ => None,
        };
        for line in started_by.into_iter().chain(using).chain(scan_line(&added)) {
            progress(line.clone());
            log.push(line);
        }
        // Read by the engine just before the first step, so the figures are
        // this run's and not whatever an open window last happened to poll.
        // A run with nothing to do is not measured.
        let before = if plan.steps.is_empty() {
            None
        } else {
            self.measure()
        };

        let mut journal = Journal::new(now());
        journal.began = Some(self.platform.marker());
        journal.ending = ending;
        journal.profile = Some(settings.profile_name.clone());
        journal.save(&self.data_dir)?;
        let mut took_effect = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            let line = match self.run_journaled(step, snapshot.power_plan.as_ref(), &mut journal) {
                Ok(line) => line,
                Err(error) => return Err(self.stop_unrecorded(journal, log, error)),
            };
            took_effect.push(line.ok && line.detail.is_none());
            progress(line.clone());
            log.push(line);
        }
        let run_report = before.and_then(|before| {
            let after = self.measure()?;
            Some(RunReport::measured(
                &before,
                &after,
                &plan.steps,
                &took_effect,
                &snapshot.processes,
            ))
        });
        let summary = journal.summary();
        let mut inner = self.lock();
        inner.journal = Some(journal);
        inner.log = log;
        inner.skipped = plan.skipped;
        inner.run_report = run_report;
        inner.recovered = false;
        Ok(summary)
    }

    /// The profile chosen for the program that started a run, if one was.
    fn profile_for_trigger(&self, ending: Option<&Ending>) -> Option<String> {
        let Some(Ending::Trigger { program }) = ending else {
            return None;
        };
        self.lock()
            .settings
            .auto_quiet
            .profile_for(program)
            .map(str::to_string)
    }

    /// A journal save failed mid-run: keep what was done where Restore can
    /// see it, and change nothing more. A hold on the PC or an ending is
    /// kept too: they are in the file already, which is what a restart reads.
    fn stop_unrecorded(&self, journal: Journal, log: Vec<LogLine>, error: CoreError) -> AppError {
        let mut inner = self.lock();
        inner.journal = (!journal.is_finished()).then_some(journal);
        inner.log = log;
        error.into()
    }

    /// Hold off sleep. Written to the journal first, like every step, so a
    /// crash leaves a run that takes it up again; but it is no entry to undo:
    /// the hold ends with this process, so nothing it does can outlive it.
    pub(super) fn hold_awake(
        &self,
        step: &Step,
        journal: &mut Journal,
    ) -> Result<LogLine, CoreError> {
        journal.awake = true;
        if let Err(error) = journal.save(&self.data_dir) {
            journal.awake = false;
            return Err(error);
        }
        Ok(match self.platform.keep_awake(true) {
            Ok(()) => LogLine {
                label: step.label(),
                ok: true,
                detail: None,
            },
            Err(error) => {
                log::warn!("{} failed: {error}", step.label());
                journal.awake = false;
                if let Err(error) = journal.save(&self.data_dir) {
                    log::warn!("removing a hold that did not happen: {error}");
                }
                LogLine {
                    label: step.label(),
                    ok: false,
                    detail: Some(error.to_string()),
                }
            }
        })
    }
}

/// The program a run waits for, or was started by, is not parked by it.
fn protect_what_it_waits_for(settings: &mut Settings, ending: Option<&Ending>) {
    if let Some(program) = ending.and_then(Ending::program) {
        settings.profile.protect(program);
    }
}

/// A run that waits for a program needs it to be there: one that is not
/// running would end the run at once.
fn check_it_is_running(
    ending: Option<&Ending>,
    processes: &[cq_core::ProcessInfo],
) -> Result<(), AppError> {
    let Some(Ending::ProgramExits { name }) = ending else {
        return Ok(());
    };
    match running(
        std::slice::from_ref(name),
        processes,
        cq_platform::current_pid(),
    ) {
        Some(_) => Ok(()),
        None => Err(AppError::new(
            "program_not_running",
            format!("{name} is not running, so there is nothing to wait for"),
        )),
    }
}

/// Said at the top of the log when there is more than one profile.
fn used_profile(settings: &Settings) -> LogLine {
    LogLine {
        label: format!("Using the {} profile", settings.profile_name),
        ok: true,
        detail: None,
    }
}

/// Said at the top of the log of a run the watch started. A llama.cpp server
/// with one model is the one thing it still stops (suspending it would free
/// nothing), so the promise of closing nothing says so when the plan has it.
fn started_because(program: &str, plan: &Plan) -> LogLine {
    let closed = if plan
        .steps
        .iter()
        .any(|step| matches!(step, Step::CloseModelServer(_)))
    {
        "Nothing is closed but a llama.cpp server with one model, which is started again when this ends, and"
    } else {
        "Nothing is closed and"
    };
    LogLine {
        label: format!("Started because {program} is running"),
        ok: true,
        detail: Some(format!(
            "{closed} cached memory is left alone, because you did not press the button"
        )),
    }
}

fn scan_line(added: &[Recommendation]) -> Option<LogLine> {
    (!added.is_empty()).then(|| LogLine {
        label: format!("Scan added {} low-risk target(s)", added.len()),
        ok: true,
        detail: Some(
            added
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    })
}
