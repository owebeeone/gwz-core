//! The retry state an endpoint keeps for the operations it serves: each
//! operation's budget, and a machine for each pool key it opens. The state is
//! per operation, as the retry plan's §5 and the reuse design's §9 keep it.
use super::Machine;
use std::collections::BTreeMap;

/// Extra setup attempts after the first when an operation names none
/// (`--max-retries`'s default; the retry plan's §5).
pub(crate) const DEFAULT_MAX_RETRIES: u32 = 3;

/// The machines of the operations an endpoint serves, by operation, then by
/// pool key `K`, whose members are `M`.
pub(crate) struct Operations<K, M> {
    operations: BTreeMap<String, Operation<K, M>>,
}
struct Operation<K, M> {
    max_retries: u32,
    keys: Vec<(K, Machine<M>)>,
}

impl<K: PartialEq + Clone, M: PartialEq> Operations<K, M> {
    pub(crate) fn new() -> Self {
        Self {
            operations: BTreeMap::new(),
        }
    }
    /// The operation's budget, which its admission installs before it opens
    /// anything. An operation never given one has the default.
    pub(crate) fn set_max_retries(&mut self, operation: &str, max_retries: u32) {
        self.entry(operation).max_retries = max_retries;
    }
    /// The machine of `key` within `operation`, made on its first use.
    pub(crate) fn machine(&mut self, operation: &str, key: &K) -> &mut Machine<M> {
        let entry = self.entry(operation);
        let index = match entry.keys.iter().position(|(known, _)| known == key) {
            Some(index) => index,
            None => {
                entry
                    .keys
                    .push((key.clone(), Machine::new(entry.max_retries)));
                entry.keys.len() - 1
            }
        };
        &mut entry.keys[index].1
    }
    /// Forgets an operation once it is finished or cancelled: the next
    /// operation starts its keys Cold.
    pub(crate) fn remove(&mut self, operation: &str) {
        self.operations.remove(operation);
    }
    pub(crate) fn clear(&mut self) {
        self.operations.clear();
    }
    fn entry(&mut self, operation: &str) -> &mut Operation<K, M> {
        self.operations
            .entry(operation.to_owned())
            .or_insert_with(|| Operation {
                max_retries: DEFAULT_MAX_RETRIES,
                keys: Vec::new(),
            })
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use super::{Decision, Outcome, Verdict, machine::Final};
        use gwz_transport::protocol::{Effect, ErrorCode, Failure, SetupFailureCause};

        fn stall() -> Failure {
            Failure {
                setup_cause: Some(SetupFailureCause::Stall),
                code: ErrorCode::Timeout,
                effect: Effect::None,
                facts: None,
            }
        }
        /// Fails `key`'s one member of `operation` once, and returns the outcome.
        fn fail_once(operations: &mut Operations<&'static str, u8>, operation: &str, key: &'static str) -> Outcome {
            let machine = operations.machine(operation, &key);
            assert_eq!(machine.decide(0), Decision::Start);
            machine.start(1);
            machine.failed(&1, Verdict::Retry, stall(), 0, 0)
        }

        #[test]
        fn each_operation_has_its_own_budget_and_each_key_its_own_machine() {
            let mut operations = Operations::new();
            operations.set_max_retries("strict", 0);
            let finished = Outcome::Finish(Final { failure: stall(), attempt: 1, attempts: 1 });
            assert_eq!(fail_once(&mut operations, "strict", "a"), finished);
            // Another key of the same operation is still Cold.
            assert_eq!(operations.machine("strict", &"b").decide(0), Decision::Start);
            // An operation that names no budget has the default, three retries.
            assert_eq!(fail_once(&mut operations, "default", "a"), Outcome::Retry);
        }

        #[test]
        fn a_removed_operation_starts_its_keys_cold_again() {
            let mut operations = Operations::new();
            operations.set_max_retries("op", 0);
            assert!(matches!(fail_once(&mut operations, "op", "a"), Outcome::Finish(_)));
            assert!(matches!(operations.machine("op", &"a").decide(0), Decision::Finish(_)));
            operations.remove("op");
            assert_eq!(operations.machine("op", &"a").decide(0), Decision::Start);
            assert_eq!(fail_once(&mut operations, "op", "a"), Outcome::Retry, "and the default budget");
        }
    }
}
