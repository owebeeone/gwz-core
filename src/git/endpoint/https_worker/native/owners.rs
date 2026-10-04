//! Start and Finish owners that stay pollable after the exchange that started them is dropped.
use super::*;

// The owned Finish stays pollable after its enclosing task disappears. Poll and
// receipt checks happen outside this shared holder's lock; the endpoint reaper
// supplies subsequent polls without introducing another process/runtime owner.
struct FinishState {
    work: Option<Work<'static, Result<(), BridgeError>>>,
    outcome: Option<Result<(), BridgeError>>,
}
#[derive(Clone)]
pub(super) struct Finishing(Arc<Mutex<FinishState>>);
impl Finishing {
    pub(super) fn new(work: Work<'static, Result<(), BridgeError>>) -> Self {
        Self(Arc::new(Mutex::new(FinishState {
            work: Some(work),
            outcome: None,
        })))
    }
    fn advance(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), ErrorCode>> {
        let work = self.0.lock().unwrap_or_else(|p| p.into_inner()).work.take();
        if let Some(mut work) = work {
            let polled = work.as_mut().poll(cx);
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            match polled {
                std::task::Poll::Pending => state.work = Some(work),
                std::task::Poll::Ready(result) => state.outcome = Some(result),
            }
        }
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match &state.outcome {
            Some(Ok(())) => std::task::Poll::Ready(Ok(())),
            Some(Err(error)) => std::task::Poll::Ready(Err(error.code)),
            None => std::task::Poll::Pending,
        }
    }
}
impl std::future::Future for Finishing {
    type Output = Result<(), ErrorCode>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.advance(cx)
    }
}
impl Probe for Finishing {
    fn confirmed(&self) -> bool {
        let _ = self.advance(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        let outcome = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .outcome
            .take();
        let confirmed = match &outcome {
            Some(Ok(())) => true,
            Some(Err(error)) => error.pending.as_ref().is_none_or(|probe| probe.confirmed()),
            None => false,
        };
        self.0.lock().unwrap_or_else(|p| p.into_inner()).outcome = outcome;
        confirmed
    }
}
// Start owns registration/launch/Hello even after enclosing preparation Drop.
// Late conversations are cancelled; no publication is available to the reaper.
struct StartState {
    work: Option<Work<'static, Result<Box<dyn Session>, BridgeError>>>,
    outcome: Option<Result<Box<dyn Session>, BridgeError>>,
    cleanup: Option<Box<dyn Probe>>,
    disposing: bool,
    disposed: bool,
}
#[derive(Clone)]
pub(super) struct Starting(Arc<Mutex<StartState>>);
impl Starting {
    pub(super) fn new(work: Work<'static, Result<Box<dyn Session>, BridgeError>>) -> Self {
        Self(Arc::new(Mutex::new(StartState {
            work: Some(work),
            outcome: None,
            cleanup: None,
            disposing: false,
            disposed: false,
        })))
    }
    fn advance(&self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), ErrorCode>> {
        let work = self.0.lock().unwrap_or_else(|p| p.into_inner()).work.take();
        if let Some(mut work) = work {
            let result = work.as_mut().poll(cx);
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            match result {
                std::task::Poll::Pending => state.work = Some(work),
                std::task::Poll::Ready(outcome) => state.outcome = Some(outcome),
            }
        }
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match &state.outcome {
            Some(Ok(_)) => std::task::Poll::Ready(Ok(())),
            Some(Err(error)) => std::task::Poll::Ready(Err(error.code)),
            None => std::task::Poll::Pending,
        }
    }
    pub(super) fn take_session(&self) -> Box<dyn Session> {
        match self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .outcome
            .take()
        {
            Some(Ok(session)) => session,
            _ => unreachable!("Start admitted a session"),
        }
    }
}
impl std::future::Future for Starting {
    type Output = Result<(), ErrorCode>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.advance(cx)
    }
}
impl Probe for Starting {
    fn confirmed(&self) -> bool {
        let _ = self.advance(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        let outcome = {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.disposing {
                return false;
            }
            let outcome = state.outcome.take();
            if outcome.is_some() {
                state.disposing = true;
            }
            outcome
        };
        if let Some(outcome) = outcome {
            let cleanup = match outcome {
                Ok(session) => session.cancel(),
                Err(error) => error.pending,
            };
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            state.cleanup = cleanup;
            state.disposing = false;
            state.disposed = true;
        }
        let (disposed, cleanup) = {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.disposing {
                return false;
            }
            state.disposing = true;
            (state.disposed, state.cleanup.take())
        };
        let confirmed = disposed && cleanup.as_ref().is_none_or(|probe| probe.confirmed());
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.cleanup = cleanup;
        state.disposing = false;
        confirmed
    }
}
