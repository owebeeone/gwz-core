use super::{IdleReactor, IdleSocket};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

struct Signal(Sender<()>);
impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}
fn waker() -> (Waker, Receiver<()>) {
    let (sender, receiver) = mpsc::channel();
    (Waker::from(Arc::new(Signal(sender))), receiver)
}
/// A connected loopback pair: (the watched client side, the server side).
fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}
/// Watches `client` and returns once its first poll has registered `waker`.
fn watch(reactor: &Arc<IdleReactor>, client: &TcpStream, waker: &Waker) -> IdleSocket {
    let socket = IdleSocket::watch(reactor, client.try_clone().unwrap()).unwrap();
    let mut cx = Context::from_waker(waker);
    assert_eq!(socket.poll_lost(&mut cx), Poll::Pending);
    socket
}
/// The reactor wakes the registered waker; nothing polls in between.
fn lost_after_wake(socket: &IdleSocket, waker: &Waker, woken: &Receiver<()>) {
    woken
        .recv_timeout(Duration::from_secs(5))
        .expect("the reactor woke the waker");
    let mut cx = Context::from_waker(waker);
    assert_eq!(socket.poll_lost(&mut cx), Poll::Ready(()));
}

#[test]
fn a_quiet_peer_is_not_lost_and_wakes_nothing() {
    let reactor = IdleReactor::start().unwrap();
    let (client, _server) = pair();
    let (waker, woken) = waker();
    let socket = watch(&reactor, &client, &waker);
    assert!(woken.recv_timeout(Duration::from_millis(100)).is_err());
    let mut cx = Context::from_waker(&waker);
    assert_eq!(socket.poll_lost(&mut cx), Poll::Pending);
}

#[test]
fn the_peer_closing_wakes_the_waker_and_is_lost() {
    let reactor = IdleReactor::start().unwrap();
    let (client, server) = pair();
    let (waker, woken) = waker();
    let socket = watch(&reactor, &client, &waker);
    drop(server);
    lost_after_wake(&socket, &waker, &woken);
}

#[test]
fn a_reset_wakes_the_waker_and_is_lost() {
    let reactor = IdleReactor::start().unwrap();
    let (client, server) = pair();
    let (waker, woken) = waker();
    let socket = watch(&reactor, &client, &waker);
    socket2::SockRef::from(&server)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    drop(server);
    lost_after_wake(&socket, &waker, &woken);
}

#[test]
fn unsolicited_bytes_are_lost_and_stay_unread() {
    let reactor = IdleReactor::start().unwrap();
    let (mut client, mut server) = pair();
    let (waker, woken) = waker();
    let socket = watch(&reactor, &client, &waker);
    server.write_all(b"x").unwrap();
    lost_after_wake(&socket, &waker, &woken);
    drop(socket);
    let mut byte = [0];
    client.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"x");
}

#[test]
fn a_loss_before_the_watch_starts_is_reported_at_once() {
    let reactor = IdleReactor::start().unwrap();
    let (client, server) = pair();
    drop(server);
    std::thread::sleep(Duration::from_millis(20));
    let (waker, woken) = waker();
    let socket = IdleSocket::watch(&reactor, client.try_clone().unwrap()).unwrap();
    let mut cx = Context::from_waker(&waker);
    if socket.poll_lost(&mut cx).is_pending() {
        lost_after_wake(&socket, &waker, &woken);
    }
}

#[test]
fn each_watch_keeps_its_reactor() {
    let reactor = IdleReactor::start().unwrap();
    let (client, _server) = pair();
    let (waker, _woken) = waker();
    let socket = watch(&reactor, &client, &waker);
    let weak = Arc::downgrade(&reactor);
    drop(reactor);
    assert!(weak.upgrade().is_some());
    drop(socket);
    assert!(weak.upgrade().is_none());
}
