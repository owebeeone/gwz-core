use super::*;

impl Prepared {
    pub(crate) fn io_timeout_ms(&self) -> u64 {
        self.io_ms
    }
    pub(super) fn request(&mut self, body: RequestBody) -> Result<Request<RequestBody>, Failure> {
        let url = self.destination.request(self.input.service);
        let mut request = Request::builder()
            .method(if https_policy::advertisement(self.input.service) {
                "GET"
            } else {
                "POST"
            })
            .uri(&url[url::Position::BeforePath..])
            .header(HOST, self.destination.authority())
            .header(ACCEPT, https_policy::response_type(self.input.service));
        if !https_policy::advertisement(self.input.service) {
            request = request.header(
                CONTENT_TYPE,
                format!(
                    "application/x-{}-request",
                    https_policy::service_name(self.input.service)
                ),
            );
        }
        if let Some(authorization) = self.authorization.take() {
            request = request.header(AUTHORIZATION, authorization);
        }
        let mut request = request
            .body(body)
            .map_err(|_| failure(ErrorCode::InvalidRequest))?;
        if let Some(value) = request.headers_mut().get_mut(AUTHORIZATION) {
            value.set_sensitive(true);
        }
        let protocol_error = self.protocol_error.clone();
        let cancel = self.lease.as_ref().unwrap().cancel.clone();
        let count = Arc::new(AtomicUsize::new(0));
        hyper::ext::on_informational(&mut request, move |response| {
            if !matches!(response.status().as_u16(), 100 | 102 | 103)
                || count.fetch_add(1, Ordering::Relaxed) >= 8
            {
                protocol_error.store(true, Ordering::Release);
                cancel.cancel();
            }
        });
        Ok(request)
    }
    pub(crate) async fn serve(
        mut self,
        stream: Stream,
        peer: Arc<MessageEndpoint>,
        cancel: CancellationToken,
    ) {
        let resource_cancel = self.lease.as_ref().unwrap().cancel.clone();
        let facts = Arc::new(Mutex::new(self.opened.facts.clone()));
        let possible = Arc::new(AtomicBool::new(false));
        let progress = self
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .lock()
            .await
            .progress
            .clone();
        let mut observed = progress.load(Ordering::Relaxed);
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        let result = {
            let work = self.run(&stream, &peer, facts.clone(), possible.clone());
            tokio::pin!(work);
            loop {
                tokio::select! {
                    result=&mut work=>break result,
                    _=cancel.cancelled()=>break Err(ErrorCode::Cancelled),
                    _=resource_cancel.cancelled()=>break Err(ErrorCode::Cancelled),
                    _=tick.tick()=>{
                        let now=progress.load(Ordering::Relaxed);let bytes=now.wrapping_sub(observed);observed=now;
                        if bytes>0 && peer.io_status().state==IoState::Network {let _=peer.record_io_progress(bytes.min(usize::MAX as u64) as usize);}
                    },
                }
            }
        };
        if let Err(mut code) = result {
            if self.protocol_error.load(Ordering::Acquire) {
                code = ErrorCode::Protocol;
            }
            let effect = if possible.load(Ordering::Acquire) {
                Effect::Possible
            } else {
                Effect::None
            };
            let facts = facts.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let _ = peer.fail_terminal(with_facts(code, effect, &facts));
        }
    }
    async fn run(
        &mut self,
        stream: &Stream,
        peer: &Arc<MessageEndpoint>,
        facts: Arc<Mutex<Facts>>,
        possible: Arc<AtomicBool>,
    ) -> Result<(), ErrorCode> {
        let mut response = if let Some(response) = self.response.take() {
            response
        } else {
            let (tx, body) = body_channel();
            let request = self.request(body).map_err(|e| e.code)?;
            *facts.lock().unwrap_or_else(|e| e.into_inner()) = self.opened.facts.clone();
            let producer = async {
                let mut buffer = vec![0; 16384];
                loop {
                    peer.set_io_state(IoState::Backpressure)
                        .map_err(|_| ErrorCode::Io)?;
                    let n = stream.read(&mut buffer).await.map_err(stream_code)?;
                    if n == 0 {
                        peer.set_io_state(IoState::Network).map_err(stream_code)?;
                        break;
                    }
                    peer.set_io_state(IoState::Network)
                        .map_err(|_| ErrorCode::Io)?;
                    tx.send(Ok(Bytes::copy_from_slice(&buffer[..n])))
                        .await
                        .map_err(|_| ErrorCode::Io)?;
                }
                drop(tx);
                Ok::<_, ErrorCode>(())
            };
            let connection = self
                .lease
                .as_ref()
                .unwrap()
                .connection
                .as_ref()
                .unwrap()
                .clone();
            let mut guard = connection.lock().await;
            let send = async {
                facts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .credential_offered = request.headers().contains_key(AUTHORIZATION);
                if self.input.service == GitService::ReceivePackExchange {
                    possible.store(true, Ordering::Release);
                }
                guard
                    .sender
                    .send_request(request)
                    .await
                    .map_err(|error| classify_hyper_error(&error))
            };
            tokio::pin!(send, producer);
            tokio::select! {
                result=&mut send=>{
                    let response=result?;
                    record_response(&response,&facts);
                    check_response(&response,self.input.service)?;
                    // A final rejection is returned immediately, even when Git is
                    // blocked writing a body. Successful responses still require
                    // an explicit EndWrite before this connection can be reused.
                    producer.await?;
                    response
                },
                result=&mut producer=>{
                    let response=send.await?;
                    record_response(&response,&facts);
                    check_response(&response,self.input.service)?;
                    result?;
                    response
                },
            }
        };
        record_response(&response, &facts);
        check_response(&response, self.input.service)?;
        loop {
            peer.set_io_state(IoState::Network).map_err(stream_code)?;
            let Some(frame) = response.body_mut().frame().await else {
                break;
            };
            let frame = frame.map_err(|_| ErrorCode::Protocol)?;
            if let Ok(data) = frame.into_data() {
                peer.record_io_progress(data.len()).map_err(stream_code)?;
                peer.set_io_state(IoState::Backpressure)
                    .map_err(stream_code)?;
                for chunk in data.chunks(16384) {
                    stream.write_all(chunk).await.map_err(stream_code)?;
                }
            }
        }
        drop(response);
        peer.set_io_state(IoState::Backpressure)
            .map_err(stream_code)?;
        stream.end_write().await.map_err(stream_code)?;
        // GET has no body; wait for the peer's explicit EndWrite before close.
        if https_policy::advertisement(self.input.service) {
            let mut unexpected = [0];
            if stream.read(&mut unexpected).await.map_err(stream_code)? != 0 {
                return Err(ErrorCode::Protocol);
            }
        }
        let until = Instant::now() + Duration::from_millis(self.cleanup_ms);
        let disposition = {
            let connection = self.lease.as_ref().unwrap().connection.as_ref().unwrap();
            let mut guard = connection.lock().await;
            if matches!(
                tokio::time::timeout_at(until, guard.sender.ready()).await,
                Ok(Ok(()))
            ) {
                Disposition::Reusable
            } else {
                Disposition::Discarded
            }
        };
        loop {
            let final_facts = facts.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match peer.complete_close(disposition, final_facts) {
                Ok(()) => {
                    self.lease
                        .take()
                        .expect("active HTTP lease")
                        .finish(disposition)
                        .map_err(|e| e.code)?;
                    return Ok(());
                }
                Err(gwz_transport::stream::Error::WouldBlock) => {}
                Err(error) => return Err(stream_code(error)),
            }
            if Instant::now() >= until {
                return Err(ErrorCode::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}
fn record_response(response: &Response<Incoming>, facts: &Mutex<Facts>) {
    let mut f = facts.lock().unwrap_or_else(|e| e.into_inner());
    f.http_status = Some(response.status().as_u16() as i64);
    if response.status() == 401 && f.credential_offered {
        f.authenticated = Some(false);
    }
}
fn check_response(response: &Response<Incoming>, service: GitService) -> Result<(), ErrorCode> {
    match https_policy::classify(response.status().as_u16(), service) {
        ResponseAction::Success => validate_content(response, service),
        ResponseAction::Fail(code) => Err(code),
        _ => Err(ErrorCode::Protocol),
    }
}
fn stream_code(error: gwz_transport::stream::Error) -> ErrorCode {
    match error {
        gwz_transport::stream::Error::Cancelled => ErrorCode::Cancelled,
        gwz_transport::stream::Error::Timeout => ErrorCode::Timeout,
        gwz_transport::stream::Error::PeerFailed { code, .. } => code,
        _ => ErrorCode::Io,
    }
}
