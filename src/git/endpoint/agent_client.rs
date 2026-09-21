//! Endpoint-local SSH-agent codec. Only identity enumeration and signing exist.
use super::agent_job::Control;
use std::{
    io::{self, Read, Write},
    sync::Arc,
};
const FRAME: usize = 1 << 20;
const BLOB: usize = 1 << 16;
pub(crate) trait Channel: Read + Write {
    fn wait(&mut self, writing: bool, control: &Control) -> io::Result<()>;
}
pub(crate) struct Agent<C> {
    channel: C,
    control: Arc<Control>,
    failed: bool,
    enumerated: bool,
}
impl<C: Channel> Agent<C> {
    pub(crate) fn new(channel: C, control: Arc<Control>) -> Self {
        Self {
            channel,
            control,
            failed: false,
            enumerated: false,
        }
    }
    pub(crate) fn identities(&mut self) -> io::Result<Vec<Vec<u8>>> {
        if self.enumerated {
            return Err(invalid());
        }
        self.enumerated = true;
        let result = (|| {
            let bytes = self.exchange(&[11])?;
            let mut input = Input(&bytes);
            if input.byte()? != 12 {
                return Err(invalid());
            }
            let count = input.number()? as usize;
            if count > 256 {
                return Err(invalid());
            }
            let mut keys = Vec::with_capacity(count);
            for _ in 0..count {
                let key = input.string(BLOB)?;
                if key.is_empty() {
                    return Err(invalid());
                }
                keys.push(key.to_vec());
                let _ = input.string(FRAME)?;
            }
            input.end()?;
            self.control.check()?;
            Ok(keys)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub(crate) fn sign(&mut self, key: &[u8], data: &[u8], method: &str) -> io::Result<Vec<u8>> {
        let result = (|| {
            let flags = match method {
                "ssh-ed25519"
                | "ecdsa-sha2-nistp256"
                | "ecdsa-sha2-nistp384"
                | "ecdsa-sha2-nistp521" => 0_u32,
                "rsa-sha2-256" => 2,
                "rsa-sha2-512" => 4,
                _ => return Err(io::ErrorKind::Unsupported.into()),
            };
            let mut request = vec![13];
            put(&mut request, key)?;
            put(&mut request, data)?;
            request.extend_from_slice(&flags.to_be_bytes());
            let bytes = self.exchange(&request)?;
            let mut response = Input(&bytes);
            if response.byte()? != 14 {
                return Err(invalid());
            }
            let mut signature = Input(response.string(BLOB + 128)?);
            response.end()?;
            if signature.string(128)? != method.as_bytes() {
                return Err(invalid());
            }
            let raw = signature.string(BLOB)?;
            if raw.is_empty() {
                return Err(invalid());
            }
            signature.end()?;
            self.control.check()?;
            Ok(raw.to_vec())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn exchange(&mut self, body: &[u8]) -> io::Result<Vec<u8>> {
        if self.failed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.control.check()?;
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(body);
        let mut offset = 0;
        while offset < frame.len() {
            self.control.check()?;
            match self.channel.write(&frame[offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => offset += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    self.channel.wait(true, &self.control)?
                }
                Err(e) => return Err(e.kind().into()),
            }
        }
        let mut header = [0; 4];
        self.read_exact(&mut header)?;
        let len = u32::from_be_bytes(header) as usize;
        if len == 0 || len > FRAME {
            return Err(invalid());
        }
        let mut response = vec![0; len];
        self.read_exact(&mut response)?;
        self.control.check()?;
        if response[0] == 5 {
            return Err(if response.len() == 1 {
                io::ErrorKind::PermissionDenied.into()
            } else {
                invalid()
            });
        }
        Ok(response)
    }
    fn read_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            self.control.check()?;
            match self.channel.read(&mut bytes[offset..]) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => offset += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) =>
                {
                    self.channel.wait(false, &self.control)?
                }
                Err(e) => return Err(e.kind().into()),
            }
        }
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid SSH agent response")
}
fn put(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > BLOB {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, len: usize) -> io::Result<&'a [u8]> {
        if len > self.0.len() {
            return Err(invalid());
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn number(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self, max: usize) -> io::Result<&'a [u8]> {
        let n = self.number()? as usize;
        if n > max {
            return Err(invalid());
        }
        self.take(n)
    }
    fn end(self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}
