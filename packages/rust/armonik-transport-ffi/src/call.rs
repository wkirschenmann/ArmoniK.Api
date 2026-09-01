use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};

use armonik_transport::grpc::{CallControl, GrpcStatusCode, RecvHalf, RecvResult, SendHalf};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::abi::{ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_status};
use crate::blob;
use crate::host::{Host, HostPtr};
use crate::runtime::{AkRuntime, Ledger};

pub(crate) const DELIVERY_CREDITS: usize = 1;

pub(crate) enum Command {
    Send(Bytes),
    EndSend,
}

struct SendWindow {
    occupied: AtomicU32,
    max: u32,
}

impl SendWindow {
    fn take(&self) -> bool {
        let mut seen = self.occupied.load(Ordering::Acquire);
        loop {
            if seen >= self.max {
                return false;
            }
            match self.occupied.compare_exchange_weak(
                seen,
                seen + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => seen = current,
            }
        }
    }

    fn release(&self) {
        self.occupied.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
struct Debt {
    payloads: AtomicU32,
    buffers: AtomicU32,
    callbacks: AtomicU32,
    terminal: AtomicBool,
}

pub(crate) struct CallState {
    ctx: HostPtr,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    control: CallControl,
    commands: mpsc::Sender<Command>,
    window: SendWindow,
    credits: Arc<Semaphore>,
    debt: Debt,
    cancelled: AtomicBool,
    progress: watch::Sender<u64>,
    over: watch::Sender<bool>,
    ended_sending: AtomicBool,
    runtime: Weak<AkRuntime>,
    handle: ak_handle,
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        ak_call_debt {
            payloads_owed: self.debt.payloads.load(Ordering::Acquire),
            buffers_lent: self.debt.buffers.load(Ordering::Acquire),
            callbacks_in_flight: self.debt.callbacks.load(Ordering::Acquire),
            terminal_delivered: self.debt.terminal.load(Ordering::Acquire) as i32,
        }
    }

    pub(crate) fn belongs_to(&self, runtime: &Weak<AkRuntime>) -> bool {
        Weak::ptr_eq(&self.runtime, runtime)
    }

    pub(crate) fn handle(&self) -> ak_handle {
        self.handle
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.moved_on();
        self.control.cancel();
    }

    fn ended_sending(&self) -> bool {
        self.ended_sending.load(Ordering::Acquire)
    }

    fn live(&self) -> bool {
        !*self.over.borrow()
    }

    pub(crate) fn lend(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if !self.live() || self.cancelled.load(Ordering::Acquire) {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        self.ledger.could_ever_fit(len)?;
        if !self.window.take() {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        }
        if let Err(status) = self.ledger.reserve(len) {
            self.window.release();
            return Err(status);
        }

        self.debt.buffers.fetch_add(1, Ordering::AcqRel);

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Arc::downgrade(self),
            data: vec![0u8; len],
        });
        let ptr = lent.data.as_mut_ptr();
        Ok(ak_buffer {
            ptr,
            len,
            owner: Box::into_raw(lent) as *mut c_void,
        })
    }

    fn took_back(&self, len: usize) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release_bytes(len);
        self.moved_on();
    }

    pub(crate) fn commit(self: &Arc<Self>, lent: Box<Lent>) -> ak_status {
        if !self.live() || self.cancelled.load(Ordering::Acquire) || self.ended_sending() {
            let _ = Box::into_raw(lent);
            return ak_status::AK_STATUS_INVALID_STATE;
        }

        let Ok(slot) = self.commands.try_reserve() else {
            let _ = Box::into_raw(lent);
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        self.took_back(lent.data.len());
        slot.send(Command::Send(Bytes::from(lent.data)));
        ak_status::AK_STATUS_OK
    }

    #[allow(clippy::boxed_local)]
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        self.took_back(lent.data.len());
        self.window.release();
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() || self.ended_sending.swap(true, Ordering::AcqRel) {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        match self.commands.try_send(Command::EndSend) {
            Ok(()) => ak_status::AK_STATUS_OK,
            Err(_) => {
                self.ended_sending.store(false, Ordering::Release);
                ak_status::AK_STATUS_INVALID_STATE
            }
        }
    }

    fn in_callback(&self, emit: impl FnOnce()) {
        self.debt.callbacks.fetch_add(1, Ordering::AcqRel);
        emit();
        self.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    fn moved_on(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    pub(crate) async fn finished(&self) {
        let mut progress = self.progress.subscribe();
        while !(self.debt.terminal.load(Ordering::Acquire)
            && self.debt.callbacks.load(Ordering::Acquire) == 0)
        {
            if progress.changed().await.is_err() {
                return;
            }
        }
    }

    fn settled(&self) -> bool {
        let debt = self.debt();
        debt.terminal_delivered == 1
            && debt.payloads_owed == 0
            && debt.buffers_lent == 0
            && debt.callbacks_in_flight == 0
    }
}

const LENT_TAG: u64 = 0x414b_5f4c_454e_5400;
const PAYLOAD_TAG: u64 = 0x414b_5f50_4159_4c00;

#[repr(C)]
pub(crate) struct Lent {
    tag: u64,
    call: Weak<CallState>,
    data: Vec<u8>,
}

impl Lent {
    pub(crate) fn call(&self) -> Option<Arc<CallState>> {
        self.call.upgrade()
    }
}

unsafe fn take_tagged<T>(owner: *mut c_void, tag: u64) -> Option<Box<T>> {
    if owner.is_null() {
        return None;
    }
    if unsafe { *(owner as *const u64) } != tag {
        return None;
    }
    Some(unsafe { Box::from_raw(owner as *mut T) })
}

pub(crate) unsafe fn take_lent(owner: *mut c_void) -> Option<Box<Lent>> {
    unsafe { take_tagged(owner, LENT_TAG) }
}

#[repr(C)]
pub(crate) struct Payload {
    tag: u64,
    data: Vec<u8>,
    call: Arc<CallState>,
    returns_credit: bool,
}

impl Drop for Payload {
    fn drop(&mut self) {
        if self.returns_credit {
            self.call.credits.add_permits(1);
        }
        self.call.debt.payloads.fetch_sub(1, Ordering::AcqRel);
        self.call.ledger.release();
        self.call.moved_on();
    }
}

pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    unsafe { take_tagged(owner, PAYLOAD_TAG) }
}

fn lend_payload(call: &Arc<CallState>, data: Vec<u8>, returns_credit: bool) -> ak_bytes {
    call.debt.payloads.fetch_add(1, Ordering::AcqRel);
    call.ledger.hold();

    let payload = Box::new(Payload {
        tag: PAYLOAD_TAG,
        data,
        call: Arc::clone(call),
        returns_credit,
    });
    let ptr = payload.data.as_ptr();
    let len = payload.data.len();
    ak_bytes {
        ptr,
        len,
        owner: Box::into_raw(payload) as *mut c_void,
    }
}

pub(crate) fn create(
    ctx: HostPtr,
    handle: ak_handle,
    runtime: &Arc<AkRuntime>,
    control: CallControl,
    max_sends_in_flight: u32,
    delivery_credits: usize,
) -> (Arc<CallState>, mpsc::Receiver<Command>) {
    let (tx, rx) = mpsc::channel(max_sends_in_flight as usize + 1);

    let state = Arc::new(CallState {
        ctx,
        host: Arc::clone(&runtime.host),
        ledger: Arc::clone(&runtime.ledger),
        control,
        commands: tx,
        window: SendWindow {
            occupied: AtomicU32::new(0),
            max: max_sends_in_flight,
        },
        credits: Arc::new(Semaphore::new(delivery_credits)),
        debt: Debt::default(),
        cancelled: AtomicBool::new(false),
        progress: watch::channel(0).0,
        over: watch::channel(false).0,
        ended_sending: AtomicBool::new(false),
        runtime: Arc::downgrade(runtime),
        handle,
    });
    (state, rx)
}

pub(crate) fn start(
    state: &Arc<CallState>,
    send: SendHalf,
    recv: RecvHalf,
    commands: mpsc::Receiver<Command>,
    spawner: &tokio::runtime::Handle,
) {
    let (writer_done, writer_is_done) = oneshot::channel();

    spawner.spawn(writer(Arc::clone(state), send, commands, writer_done));
    spawner.spawn(reader(Arc::clone(state), recv, writer_is_done));
    spawner.spawn(reclaim(Arc::clone(state)));
}

async fn writer(
    state: Arc<CallState>,
    send: SendHalf,
    mut commands: mpsc::Receiver<Command>,
    done: oneshot::Sender<()>,
) {
    let mut send = Some(send);
    let mut over = state.over.subscribe();
    let mut draining = false;

    loop {
        let command = if draining {
            commands.recv().await
        } else {
            tokio::select! {
                biased;
                _ = over.wait_for(|over| *over) => {
                    commands.close();
                    draining = true;
                    continue;
                }
                command = commands.recv() => command,
            }
        };

        let Some(command) = command else { break };
        match command {
            Command::Send(bytes) => {
                if let Some(half) = send.as_mut() {
                    let _ = half.send_message(bytes).await;
                }
                state.window.release();
                state.debt.callbacks.fetch_add(1, Ordering::AcqRel);
                state
                    .host
                    .signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE);
                state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
                state.moved_on();
            }
            Command::EndSend => {
                if let Some(half) = send.take() {
                    let _ = half.end_send().await;
                }
            }
        }
    }

    let _ = done.send(());
}

async fn reader(state: Arc<CallState>, mut recv: RecvHalf, writer_is_done: oneshot::Receiver<()>) {
    let head = recv.recv_initial_metadata().await.unwrap_or_default();
    let delivered_head = deliver(
        &state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        blob::encode_metadata(&head),
    )
    .await;

    let status = if !delivered_head {
        recv.next_message().await.ok().and_then(terminal_of)
    } else {
        loop {
            match recv.next_message().await {
                Ok(RecvResult::Message(message)) => {
                    if !deliver(
                        &state,
                        ak_event_kind::AK_EVENT_MESSAGE,
                        message.data.to_vec(),
                    )
                    .await
                    {
                        break recv.next_message().await.ok().and_then(terminal_of);
                    }
                }
                Ok(RecvResult::End(status)) => break Some(status),
                Err(_) => break None,
            }
        }
    };

    state.over.send_replace(true);
    let _ = writer_is_done.await;

    let (code, message, trailers) = match status {
        Some(status) => (
            status.code as i32,
            status.message,
            blob::encode_metadata(&status.trailing_metadata),
        ),
        None => (
            GrpcStatusCode::Cancelled as i32,
            String::from("the call was cancelled"),
            blob::encode_metadata(&Default::default()),
        ),
    };

    let payload = lend_payload(&state, status_payload(&message, &trailers), false);
    state.debt.callbacks.fetch_add(1, Ordering::AcqRel);
    state
        .host
        .deliver(state.ctx, ak_event_kind::AK_EVENT_STATUS, payload, code);
    state.debt.terminal.store(true, Ordering::Release);
    state.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
    state.moved_on();
}

fn terminal_of(result: RecvResult) -> Option<armonik_transport::grpc::GrpcStatus> {
    match result {
        RecvResult::End(status) => Some(status),
        RecvResult::Message(_) => None,
    }
}

fn status_payload(message: &str, trailers: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + message.len() + trailers.len());
    out.extend_from_slice(&(message.len() as u32).to_ne_bytes());
    out.extend_from_slice(message.as_bytes());
    out.extend_from_slice(trailers);
    out
}

async fn deliver(state: &Arc<CallState>, kind: ak_event_kind, data: Vec<u8>) -> bool {
    let credits = Arc::clone(&state.credits);
    let permit = tokio::select! {
        biased;
        permit = credits.acquire() => permit,
        () = wait_for_cancel(state) => return false,
    };
    match permit {
        Ok(permit) => permit.forget(),
        Err(_) => return false,
    }

    let payload = lend_payload(state, data, true);
    state.in_callback(|| state.host.deliver(state.ctx, kind, payload, 0));
    true
}

async fn wait_for_cancel(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    loop {
        if state.cancelled.load(Ordering::Acquire) {
            return;
        }
        if progress.changed().await.is_err() {
            return;
        }
    }
}

async fn reclaim(state: Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    while !state.settled() {
        if progress.changed().await.is_err() {
            return;
        }
    }
    if let Some(runtime) = state.runtime.upgrade() {
        runtime.reclaim_call(state.handle);
    }
}
