use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};

use armonik_transport::grpc::{
    CallControl, CallStartOptions, GrpcStatus, Metadata, RecvHalf, RecvResult, SendHalf,
};
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

use crate::abi::{ak_buffer, ak_bytes, ak_call_debt, ak_event_kind, ak_handle, ak_status};
use crate::blob;
use crate::channel::AkChannel;
use crate::host::{Host, HostPtr};
use crate::ledger::Ledger;
use crate::tables;

pub(crate) struct CallServices<'a> {
    pub(crate) host: &'a Arc<Host>,
    pub(crate) ledger: &'a Arc<Ledger>,
    pub(crate) spawner: &'a tokio::runtime::Handle,
}

pub(crate) enum Command {
    Send(Bytes),
    EndSend,
}

#[derive(Default)]
struct Debt {
    payloads: AtomicU32,
    buffers: AtomicU32,
    callbacks: AtomicU32,
    terminal: AtomicBool,
}

impl Debt {
    fn quiet(&self) -> bool {
        self.terminal.load(Ordering::Acquire) && self.callbacks.load(Ordering::Acquire) == 0
    }

    fn settled(&self) -> bool {
        self.quiet()
            && self.payloads.load(Ordering::Acquire) == 0
            && self.buffers.load(Ordering::Acquire) == 0
    }

    fn as_abi(&self) -> ak_call_debt {
        ak_call_debt {
            payloads_owed: self.payloads.load(Ordering::Acquire),
            buffers_lent: self.buffers.load(Ordering::Acquire),
            callbacks_in_flight: self.callbacks.load(Ordering::Acquire),
            terminal_delivered: self.terminal.load(Ordering::Acquire) as i32,
        }
    }
}

pub(crate) struct CallState {
    ctx: HostPtr,
    host: Arc<Host>,
    ledger: Arc<Ledger>,
    control: CallControl,
    commands: mpsc::Sender<Command>,
    window: Semaphore,
    credits: Semaphore,
    debt: Debt,
    cancelled: AtomicBool,
    progress: watch::Sender<u64>,
    over: watch::Sender<bool>,
    ended_sending: AtomicBool,
    handle: ak_handle,
    channel: ak_handle,
}

impl CallState {
    pub(crate) fn debt(&self) -> ak_call_debt {
        self.debt.as_abi()
    }

    pub(crate) fn belongs_to_channel(&self, channel: ak_handle) -> bool {
        self.channel == channel
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.announce();
        self.control.cancel();
    }

    fn ended_sending(&self) -> bool {
        self.ended_sending.load(Ordering::Acquire)
    }

    fn accepts_work(&self) -> bool {
        self.live() && !self.cancelled.load(Ordering::Acquire)
    }

    fn live(&self) -> bool {
        !*self.over.borrow()
    }

    pub(crate) fn lend(self: &Arc<Self>, len: usize) -> Result<ak_buffer, ak_status> {
        if !self.accepts_work() {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        // One buffer at a time per call, whatever the send window admits: the host names a buffer
        // by its pointer, so two out at once would be two the ABI cannot tell apart.
        if self.debt.buffers.load(Ordering::Acquire) > 0 {
            return Err(ak_status::AK_STATUS_INVALID_STATE);
        }
        self.ledger.could_ever_fit(len)?;
        let Ok(slot) = self.window.try_acquire() else {
            return Err(ak_status::AK_STATUS_SLOT_BUSY);
        };
        self.ledger.hold_bytes(len)?;

        // Forgotten, not dropped: the permit is spent for as long as the host holds the buffer, and
        // it is the WRITE_DONE that gives it back once the message has left.
        slot.forget();

        self.debt.buffers.fetch_add(1, Ordering::AcqRel);

        let mut lent = Box::new(Lent {
            tag: LENT_TAG,
            call: Arc::downgrade(self),
            data: arena(len)?,
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

    fn handed_over(&self) {
        self.debt.buffers.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    fn payload_returned(&self, returns_credit: bool) {
        if returns_credit {
            self.credits.add_permits(1);
        }
        self.debt.payloads.fetch_sub(1, Ordering::AcqRel);
        self.ledger.release();
        self.moved_on();
    }

    pub(crate) fn commit(self: &Arc<Self>, lent: Box<Lent>) -> ak_status {
        if !self.accepts_work() || self.ended_sending() {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        }

        let Ok(slot) = self.commands.try_reserve() else {
            return keep(lent, ak_status::AK_STATUS_INVALID_STATE);
        };
        self.handed_over();
        slot.send(Command::Send(Bytes::from(lent.data)));
        ak_status::AK_STATUS_OK
    }

    #[allow(clippy::boxed_local)]
    pub(crate) fn give_back(&self, lent: Box<Lent>) {
        self.took_back(lent.data.len());
        self.window.add_permits(1);
    }

    pub(crate) fn end_send(&self) -> ak_status {
        if !self.live() {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        let Ok(slot) = self.commands.try_reserve() else {
            return ak_status::AK_STATUS_INVALID_STATE;
        };
        if self.ended_sending.swap(true, Ordering::AcqRel) {
            return ak_status::AK_STATUS_INVALID_STATE;
        }
        slot.send(Command::EndSend);
        ak_status::AK_STATUS_OK
    }

    fn in_callback(&self, emit: impl FnOnce()) {
        self.debt.callbacks.fetch_add(1, Ordering::AcqRel);
        emit();
        self.debt.callbacks.fetch_sub(1, Ordering::AcqRel);
        self.moved_on();
    }

    fn moved_on(&self) {
        if self.debt.terminal.load(Ordering::Acquire) {
            self.announce();
        }
    }

    fn announce(&self) {
        self.progress.send_modify(|version| *version += 1);
    }

    pub(crate) async fn finished(&self) {
        let mut progress = self.progress.subscribe();
        let _ = progress.wait_for(|_| self.debt.quiet()).await;
    }
}

// Written into the boxes the host is given a pointer to, and checked before either is read back:
// what comes back over the ABI is whatever the host passed, and the tag is all that tells a buffer
// from a payload, or either from a pointer this library never handed out.
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

pub(crate) fn keep(lent: Box<Lent>, status: ak_status) -> ak_status {
    let _ = Box::into_raw(lent);
    status
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
    data: Bytes,
    call: Arc<CallState>,
    returns_credit: bool,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.call.payload_returned(self.returns_credit);
    }
}

pub(crate) unsafe fn take_payload(owner: *mut c_void) -> Option<Box<Payload>> {
    unsafe { take_tagged(owner, PAYLOAD_TAG) }
}

fn arena(len: usize) -> Result<Vec<u8>, ak_status> {
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| ak_status::AK_STATUS_INTERNAL)?;
    data.resize(len, 0);
    Ok(data)
}

fn lend_payload(call: &Arc<CallState>, data: Bytes, returns_credit: bool) -> ak_bytes {
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

fn create(
    ctx: HostPtr,
    handle: ak_handle,
    channel: ak_handle,
    services: &CallServices<'_>,
    control: CallControl,
    max_sends_in_flight: usize,
    delivery_credits: usize,
) -> (Arc<CallState>, mpsc::Receiver<Command>) {
    let (tx, rx) = mpsc::channel(max_sends_in_flight + 1);

    let state = Arc::new(CallState {
        ctx,
        host: Arc::clone(services.host),
        ledger: Arc::clone(services.ledger),
        control,
        commands: tx,
        window: Semaphore::new(max_sends_in_flight),
        credits: Semaphore::new(delivery_credits),
        debt: Debt::default(),
        cancelled: AtomicBool::new(false),
        progress: watch::channel(0).0,
        over: watch::channel(false).0,
        ended_sending: AtomicBool::new(false),
        channel,
        handle,
    });
    (state, rx)
}

fn start(
    state: &Arc<CallState>,
    send: SendHalf,
    recv: RecvHalf,
    commands: mpsc::Receiver<Command>,
    spawner: &tokio::runtime::Handle,
) {
    let (writer_done, writer_is_done) = oneshot::channel();

    spawner.spawn(writer(Arc::clone(state), send, commands, writer_done));
    spawner.spawn(reader(Arc::clone(state), recv, writer_is_done));
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
                let charged = bytes.len();
                if let Some(half) = send.as_mut() {
                    let _ = half.send_message(bytes).await;
                }
                // Given back inside the callback, so a host that lends again on WRITE_DONE finds the
                // room the message it just sent freed rather than a refusal it cannot explain.
                state.in_callback(|| {
                    state.window.add_permits(1);
                    state.ledger.release_bytes(charged);
                    state
                        .host
                        .signal(state.ctx, ak_event_kind::AK_EVENT_WRITE_DONE)
                });
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
    let head = match recv.recv_initial_metadata().await {
        Ok(metadata) => blob::encode_metadata(metadata),
        Err(_) => blob::encode_metadata(&Metadata::new()),
    };
    let delivered_head = deliver(
        &state,
        ak_event_kind::AK_EVENT_INITIAL_METADATA,
        Bytes::from(head),
    )
    .await;

    let status = if !delivered_head {
        GrpcStatus::cancelled()
    } else {
        loop {
            if state.cancelled.load(Ordering::Acquire) {
                break GrpcStatus::cancelled();
            }

            match recv.next_message().await {
                Ok(RecvResult::Message(message)) => {
                    if !deliver(&state, ak_event_kind::AK_EVENT_MESSAGE, message.data).await {
                        break GrpcStatus::cancelled();
                    }
                }
                Ok(RecvResult::End(status)) => break status,
                Err(_) => break GrpcStatus::cancelled(),
            }
        }
    };

    state.over.send_replace(true);
    let _ = writer_is_done.await;

    {
        let payload = lend_payload(
            &state,
            Bytes::from(blob::status_payload(
                &status.message,
                &status.trailing_metadata,
            )),
            // The terminal was never charged a delivery credit, so consuming it returns none.
            false,
        );
        state.in_callback(|| {
            state.host.deliver(
                state.ctx,
                ak_event_kind::AK_EVENT_STATUS,
                payload,
                status.code.as_i32(),
            );
            state.debt.terminal.store(true, Ordering::Release);

            crate::lifecycle::call_reached_terminal(state.channel);
        });
    }

    drop(recv);

    reclaim(&state).await;
}

async fn reclaim(state: &Arc<CallState>) {
    let mut progress = state.progress.subscribe();
    let _ = progress.wait_for(|_| state.debt.settled()).await;
    crate::lifecycle::call_settled(state.handle);
}

async fn deliver(state: &Arc<CallState>, kind: ak_event_kind, data: Bytes) -> bool {
    let permit = tokio::select! {
        biased;
        permit = state.credits.acquire() => permit,
        () = wait_for_cancel(state) => return false,
    };
    // Forgotten like the send window's: the credit is spent until the host consumes the payload,
    // and `ak_event_consumed` is what gives it back.
    match permit {
        Ok(permit) => permit.forget(),
        Err(_) => return false,
    }

    let payload = lend_payload(state, data, true);
    state.in_callback(|| state.host.deliver(state.ctx, kind, payload, 0));
    true
}

async fn wait_for_cancel(state: &CallState) {
    let mut progress = state.progress.subscribe();
    let _ = progress
        .wait_for(|_| state.cancelled.load(Ordering::Acquire))
        .await;
}

pub(crate) fn start_on(
    channel: &Arc<AkChannel>,
    channel_handle: ak_handle,
    services: &CallServices<'_>,
    method: &str,
    metadata: Metadata,
    ctx: HostPtr,
) -> Result<ak_handle, ak_status> {
    channel.join()?;

    let mut options = CallStartOptions::new(method);
    options.metadata = metadata;

    let grpc_call = match channel.grpc.start_call(options) {
        Ok(call) => call,
        Err(error) => {
            channel.leave();
            return Err(ak_status::from(error));
        }
    };

    let (send, recv, control) = grpc_call.split();
    let (handle, (state, commands)) = tables::calls().insert_with(|handle| {
        let (state, commands) = create(
            ctx,
            handle,
            channel_handle,
            services,
            control,
            channel.max_sends_in_flight,
            channel.delivery_credits,
        );
        (Arc::clone(&state), (state, commands))
    });

    start(&state, send, recv, commands, services.spawner);
    Ok(handle)
}
