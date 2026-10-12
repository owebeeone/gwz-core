//! libssh2 on Windows CNG ends about one key exchange in a hundred on its own side (`Session(-8)`, "Unable to
//! exchange encryption keys"; the server saw its own half finish and then the client close, whatever key exchange
//! method or host key algorithm was in use: `ssh_fixture::handshaken`). The transport's driver retries it. The
//! native route of a differential test is libgit2's own connection, which no retry of the transport's reaches, and
//! nothing the transport does can cause it, so an oracle that ended that way answered nothing about trust: the test
//! connects afresh, for this one message only. Any other outcome, and the last attempt's, is the answer.

/// libssh2's text for `Session(-8)`, as libgit2 passes it on.
pub(super) const KEX_FAILURE: &str = "Unable to exchange encryption keys";

/// How many connections [`retrying`] makes before it gives up (`ssh_fixture::HANDSHAKE_ATTEMPTS`).
pub(super) const ATTEMPTS: usize = 6;

/// Whether `message` is libgit2's report of a key exchange that libssh2 ended on its side.
pub(super) fn is_key_exchange_failure(message: &str) -> bool {
    message.contains(KEX_FAILURE)
}

/// Runs `attempt` with its number (0 first) until `failed` does not hold of its outcome, or `ATTEMPTS` have been
/// made; the last outcome is returned. For an outcome that carries more than the result.
pub(super) fn retrying_with<R>(
    failed: impl Fn(&R) -> bool,
    mut attempt: impl FnMut(usize) -> R,
) -> R {
    let mut number = 0;
    loop {
        let outcome = attempt(number);
        number += 1;
        if !failed(&outcome) || number >= ATTEMPTS {
            return outcome;
        }
    }
}

/// [`retrying_with`] for a result that failed with a key-exchange failure.
pub(super) fn retrying<T>(attempt: impl FnMut(usize) -> Result<T, String>) -> Result<T, String> {
    retrying_with(
        |result| {
            result
                .as_ref()
                .is_err_and(|message| is_key_exchange_failure(message))
        },
        attempt,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kex() -> String {
        format!("GitCommandFailed: failed to start SSH session: {KEX_FAILURE}")
    }

    #[test]
    fn a_key_exchange_failure_connects_afresh_until_it_does_not_happen() {
        let mut seen = Vec::new();
        let result = retrying(|number| {
            seen.push(number);
            if number < 2 { Err(kex()) } else { Ok(number) }
        });
        assert_eq!(result, Ok(2));
        assert_eq!(seen, [0, 1, 2]);
    }

    #[test]
    fn a_key_exchange_that_never_works_is_the_last_attempts_answer() {
        let mut attempts = 0;
        let result: Result<(), String> = retrying(|_| {
            attempts += 1;
            Err(kex())
        });
        assert_eq!(result, Err(kex()));
        assert_eq!(attempts, ATTEMPTS);
    }

    #[test]
    fn every_other_outcome_is_final_at_the_first_attempt() {
        for outcome in [
            Ok(7),
            Err("GitCommandFailed: remote host key mismatch".to_string()),
            Err("GitCommandFailed: failed to start SSH session: Failed getting banner".to_string()),
        ] {
            let mut attempts = 0;
            let result = retrying(|_| {
                attempts += 1;
                outcome.clone()
            });
            assert_eq!((result, attempts), (outcome, 1));
        }
    }

    #[test]
    fn an_outcome_with_more_than_the_result_is_retried_on_its_result() {
        let mut seen = Vec::new();
        let outcome = retrying_with(
            |(result, _): &(Result<u8, String>, &str)| {
                result.as_ref().is_err_and(|m| is_key_exchange_failure(m))
            },
            |number| {
                seen.push(number);
                if number == 0 {
                    (Err(kex()), "first")
                } else {
                    (Ok(1), "second")
                }
            },
        );
        assert_eq!((outcome, seen), ((Ok(1), "second"), vec![0, 1]));
    }
}
