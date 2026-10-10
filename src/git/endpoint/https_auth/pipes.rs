//! The helper's input and diagnostic pipes: neither can stall the lookup.
use super::*;

/// Writes the lookup request to the helper's input and closes it.
///
/// A helper may exit without reading its input. Git ignores SIGPIPE while it
/// writes to a credential helper, so a broken pipe here is not a failure: the
/// helper's exit status and output decide the lookup. Any other write error is.
pub(super) async fn write_request<W>(mut input: W, request: &[u8]) -> Result<(), AuthError>
where
    W: AsyncWrite + Unpin,
{
    let written = match input.write_all(request).await {
        Ok(()) => input.shutdown().await,
        Err(error) => Err(error),
    };
    match written {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(AuthError::Pipe(error.kind()))
        }
        _ => Ok(()),
    }
}

pub(super) async fn discard_stderr<R: AsyncRead + Unpin>(mut reader: R) -> Result<(), AuthError> {
    let mut scratch = SecretBuffer(vec![0; 4096]);
    loop {
        if reader
            .read(&mut scratch.0)
            .await
            .map_err(|error| AuthError::Pipe(error.kind()))?
            == 0
        {
            return Ok(());
        }
    }
}
