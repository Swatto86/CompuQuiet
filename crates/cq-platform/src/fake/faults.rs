//! Failures the fake machine can be told to produce, for the engine's tests
//! and the acceptance suite: a call that is refused, one that needs
//! administrator rights, and one that times out with its outcome unknown.

use super::{Fake, State};
use crate::error::{PlatformError, Result};

/// A platform call that can be made to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    StopService,
    StartService,
    Suspend,
    Resume,
    Close,
    Launch,
    SetPower,
    RestorePower,
    PurgeMemory,
    KeepAwake,
}

const CALLS: [(&str, Call); 10] = [
    ("stop_service", Call::StopService),
    ("start_service", Call::StartService),
    ("suspend", Call::Suspend),
    ("resume", Call::Resume),
    ("close", Call::Close),
    ("launch", Call::Launch),
    ("set_power", Call::SetPower),
    ("restore_power", Call::RestorePower),
    ("purge_memory", Call::PurgeMemory),
    ("keep_awake", Call::KeepAwake),
];

impl Call {
    /// The call named as the acceptance suite spells it, `stop_service`.
    pub fn parse(name: &str) -> Option<Call> {
        CALLS
            .iter()
            .find_map(|(spelling, call)| (*spelling == name).then_some(*call))
    }

    fn name(self) -> &'static str {
        CALLS
            .iter()
            .find_map(|(spelling, call)| (*call == self).then_some(*spelling))
            .unwrap_or("call")
    }
}

/// How a call fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Refused with a plain error; nothing happens.
    Refused,
    /// Administrator rights are needed; nothing happens.
    NeedsElevation,
    /// Gave up waiting with the outcome unknown. The effect still lands, as
    /// a slow service does when it finally stops after the wait ended.
    TimedOut,
}

impl Failure {
    /// The failure named as the acceptance suite spells it, `timed_out`.
    pub fn parse(name: &str) -> Option<Failure> {
        match name {
            "refused" => Some(Failure::Refused),
            "needs_elevation" => Some(Failure::NeedsElevation),
            "timed_out" => Some(Failure::TimedOut),
            _ => None,
        }
    }

    fn error(self, call: Call, target: &str) -> PlatformError {
        let what = format!("{} {target}", call.name());
        match self {
            Failure::Refused => PlatformError::Other(format!("the fake machine refused {what}")),
            Failure::NeedsElevation => PlatformError::NeedsElevation,
            Failure::TimedOut => PlatformError::TimedOut(format!("{what} did not finish in time")),
        }
    }
}

pub(super) struct Fault {
    call: Call,
    /// Lower-case; a service name, a process name, an executable's file name
    /// or a power plan id. `None` matches every target.
    target: Option<String>,
    failure: Failure,
}

impl State {
    /// Run `effect` unless a fault refuses the call. A timeout runs it and
    /// still reports failure, its result dropped.
    pub(super) fn guarded<T>(
        &mut self,
        call: Call,
        target: &str,
        effect: impl FnOnce(&mut State) -> Result<T>,
    ) -> Result<T> {
        let target = target.to_ascii_lowercase();
        let failure = self
            .faults
            .iter()
            .find(|fault| {
                fault.call == call && fault.target.as_ref().is_none_or(|wanted| *wanted == target)
            })
            .map(|fault| fault.failure);
        match failure {
            None => effect(self),
            Some(Failure::TimedOut) => {
                effect(self)?;
                Err(Failure::TimedOut.error(call, &target))
            }
            Some(failure) => Err(failure.error(call, &target)),
        }
    }
}

impl Fake {
    /// Make `call` fail on `target` (every target when `None`) until
    /// [`Self::heal`].
    pub fn fail(&self, call: Call, target: Option<&str>, failure: Failure) {
        self.lock().faults.push(Fault {
            call,
            target: target.map(str::to_ascii_lowercase),
            failure,
        });
    }

    /// Stop failing: the machine works again.
    pub fn heal(&self) {
        self.lock().faults.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Platform;
    use cq_core::{PowerPlan, ServiceState};

    fn state_of(fake: &Fake, name: &str) -> Option<ServiceState> {
        let snapshot = fake.snapshot(&[name.to_string()]).ok()?;
        snapshot.services.first().map(|service| service.state)
    }

    #[test]
    fn a_refused_call_changes_nothing_and_a_timed_out_one_still_lands() {
        let fake = Fake::new();
        fake.fail(Call::StopService, Some("sysmain"), Failure::Refused);
        fake.fail(Call::StopService, Some("WSearch"), Failure::TimedOut);
        fake.fail(Call::StartService, None, Failure::NeedsElevation);

        assert!(matches!(
            fake.stop_service("SysMain"),
            Err(PlatformError::Other(_))
        ));
        assert_eq!(state_of(&fake, "SysMain"), Some(ServiceState::Running));

        assert!(matches!(
            fake.stop_service("wsearch"),
            Err(PlatformError::TimedOut(_))
        ));
        assert_eq!(state_of(&fake, "WSearch"), Some(ServiceState::Stopped));

        fake.stop_service("Spooler").unwrap();
        assert!(fake.start_service("Spooler").unwrap_err().needs_elevation());
        assert_eq!(state_of(&fake, "Spooler"), Some(ServiceState::Stopped));

        fake.heal();
        fake.stop_service("SysMain").unwrap();
        fake.start_service("Spooler").unwrap();
    }

    #[test]
    fn a_plan_the_machine_does_not_have_falls_back_to_balanced() {
        let fake = Fake::new();
        let gone = PowerPlan {
            id: "oem-tuned".into(),
            name: "OEM tuned".into(),
        };
        assert_eq!(fake.restore_power(&gone).unwrap().id, "balanced");
        let performance = PowerPlan {
            id: "performance".into(),
            name: "High performance".into(),
        };
        assert_eq!(fake.restore_power(&performance).unwrap(), performance);
    }

    #[test]
    fn calls_and_failures_are_named_as_the_suite_spells_them() {
        assert_eq!(Call::parse("stop_service"), Some(Call::StopService));
        assert_eq!(Call::parse("restore_power"), Some(Call::RestorePower));
        assert_eq!(Call::parse("nonsense"), None);
        assert_eq!(Failure::parse("timed_out"), Some(Failure::TimedOut));
        assert_eq!(
            Failure::parse("needs_elevation"),
            Some(Failure::NeedsElevation)
        );
        assert_eq!(Failure::parse("nonsense"), None);
    }
}
