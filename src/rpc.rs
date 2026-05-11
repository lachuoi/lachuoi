// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::task::{RunRequest, LogMessage, SystemMetrics};
use futures_util::{Stream, Sink, StreamExt, SinkExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use pin_project::pin_project;
use tokio::sync::mpsc;

#[tarpc::service]
pub trait MasterService {
    /// Worker reports execution logs back to Master
    async fn log(msg: LogMessage);
    
    /// Worker reports system metrics
    async fn report_metrics(metrics: SystemMetrics);
    
    /// Worker requests a WASM binary from Master
    async fn get_wasm(path: String) -> Option<Vec<u8>>;
    
    /// Worker signals that a task has started
    async fn task_started(task_id: i64, task_name: String);
    
    /// Worker signals the result of a task execution
    async fn task_result(task_id: i64, log_id: Uuid, success: bool, error: Option<String>);

    /// Task (via Worker) gets values from KV store. Returns a list of values.
    async fn get_key(task_id: i64, token: String, key: String) -> Vec<String>;

    /// Task (via Worker) sets a value in KV store. Keys can be duplicated.
    async fn set_key(task_id: i64, token: String, key: String, value: String);
}

#[tarpc::service]
pub trait WorkerService {
    /// Master commands Worker to run a task
    async fn run_task(req: RunRequest);
    
    /// Master sends initial configuration to Worker
    async fn bootstrap(config_toml: String, wasm_paths: Vec<String>);
}

/// A transport that carries tarpc messages over WebSockets
#[pin_project]
pub struct WsTransport<S, In, Out, M, E> {
    #[pin]
    inner: S,
    to_msg: fn(Vec<u8>) -> M,
    from_msg: fn(M) -> Option<Vec<u8>>,
    _phantom: std::marker::PhantomData<(In, Out, E)>,
}

impl<S, In, Out, M, E> WsTransport<S, In, Out, M, E> {
    pub fn new(inner: S, to_msg: fn(Vec<u8>) -> M, from_msg: fn(M) -> Option<Vec<u8>>) -> Self {
        Self {
            inner,
            to_msg,
            from_msg,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<S, In, Out, M, E> Stream for WsTransport<S, In, Out, M, E>
where
    S: Stream<Item = Result<M, E>> + Unpin,
    In: for<'a> Deserialize<'a>,
    E: std::fmt::Display,
{
    type Item = Result<In, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        loop {
            match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(msg))) => {
                    if let Some(bin) = (this.from_msg)(msg) {
                        match bincode::deserialize(&bin) {
                            Ok(msg) => return Poll::Ready(Some(Ok(msg))),
                            Err(e) => return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))),
                        }
                    } else {
                        // Skip non-binary messages and continue polling
                        continue;
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S, In, Out, M, E> Sink<Out> for WsTransport<S, In, Out, M, E>
where
    S: Sink<M, Error = E> + Unpin,
    Out: Serialize,
    E: std::fmt::Display,
{
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_ready(cx).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }

    fn start_send(self: Pin<&mut Self>, item: Out) -> Result<(), Self::Error> {
        let this = self.project();
        let bin = bincode::serialize(&item).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let msg = (this.to_msg)(bin);
        this.inner.start_send(msg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_flush(cx).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
    }
}

/// Unified message for bidirectional RPC
#[derive(Serialize, Deserialize)]
pub enum BidiMessage<A, B> {
    A(A),
    B(B),
}

pub fn multiplex<S, AIn, AOut, BIn, BOut>(
    stream: S,
) -> (
    impl Stream<Item = Result<AIn, std::io::Error>> + Sink<AOut, Error = std::io::Error>,
    impl Stream<Item = Result<BIn, std::io::Error>> + Sink<BOut, Error = std::io::Error>,
)
where
    S: Stream<Item = Result<BidiMessage<AIn, BIn>, std::io::Error>>
        + Sink<BidiMessage<AOut, BOut>, Error = std::io::Error>
        + Send
        + 'static,
    AIn: Send + 'static,
    BIn: Send + 'static,
    AOut: Send + 'static,
    BOut: Send + 'static,
{
    let (mut sink, mut stream) = stream.split();
    let (a_tx, a_rx) = mpsc::unbounded_channel::<Result<AIn, std::io::Error>>();
    let (b_tx, b_rx) = mpsc::unbounded_channel::<Result<BIn, std::io::Error>>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<BidiMessage<AOut, BOut>>();

    // Reading task
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(BidiMessage::A(a)) => {
                    let _ = a_tx.send(Ok(a));
                }
                Ok(BidiMessage::B(b)) => {
                    let _ = b_tx.send(Ok(b));
                }
                Err(e) => {
                    let _ = a_tx.send(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())));
                    let _ = b_tx.send(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())));
                    break;
                }
            }
        }
    });

    // Writing task
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let a_transport = ChannelTransport {
        rx: a_rx,
        tx: out_tx.clone(),
        wrap: BidiMessage::A,
        _phantom: std::marker::PhantomData,
    };

    let b_transport = ChannelTransport {
        rx: b_rx,
        tx: out_tx,
        wrap: BidiMessage::B,
        _phantom: std::marker::PhantomData,
    };

    (a_transport, b_transport)
}

#[pin_project]
struct ChannelTransport<In, Out, OutOuter, F> {
    rx: mpsc::UnboundedReceiver<Result<In, std::io::Error>>,
    tx: mpsc::UnboundedSender<OutOuter>,
    wrap: F,
    _phantom: std::marker::PhantomData<Out>,
}

impl<In, Out, OutOuter, F> Stream for ChannelTransport<In, Out, OutOuter, F> {
    type Item = Result<In, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().rx.poll_recv(cx)
    }
}

impl<In, Out, OutOuter, F> Sink<Out> for ChannelTransport<In, Out, OutOuter, F>
where
    F: Fn(Out) -> OutOuter,
{
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Out) -> Result<(), Self::Error> {
        let this = self.project();
        let wrapped = (this.wrap)(item);
        this.tx
            .send(wrapped)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Multiplexer closed"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}
