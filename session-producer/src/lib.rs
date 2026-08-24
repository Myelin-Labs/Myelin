// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Bounded block-production policy for a continuous Myelin session.
//!
//! This crate decides when a transaction window closes. It neither executes
//! Cell transitions nor manufactures finality. A [`CandidateCommitter`]
//! implementation must prepare the exact session block, obtain and verify its
//! genesis-bound finality proof, and atomically advance the durable head.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot, watch};

/// Automatic policy used to close candidate transaction windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionTrigger {
    /// Close a bounded batch as soon as transactions are available.
    Instant,
    /// Close a batch on a fixed cadence. Empty production is an explicit opt-in.
    Interval { block_time: Duration, produce_empty: bool },
    /// After the first available transaction, keep one candidate open for
    /// `period`, accumulating newly available work. Idle sessions stay idle.
    Open { period: Duration },
    /// Disable automatic production while retaining the manual API.
    Never,
}

/// Stable, application-neutral configuration form for [`ProductionTrigger`].
///
/// Millisecond fields keep JSON/TOML/RON adapters independent from Rust's
/// `Duration` representation. Applications choose a policy; they do not
/// reimplement its timing semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProductionTriggerConfig {
    Instant,
    Interval {
        block_time_ms: u64,
        #[serde(default)]
        produce_empty: bool,
    },
    Open {
        period_ms: u64,
    },
    Never,
}

impl TryFrom<ProductionTriggerConfig> for ProductionTrigger {
    type Error = ProducerError;

    fn try_from(config: ProductionTriggerConfig) -> Result<Self, Self::Error> {
        let trigger = match config {
            ProductionTriggerConfig::Instant => Self::Instant,
            ProductionTriggerConfig::Interval { block_time_ms, produce_empty } => {
                Self::Interval { block_time: Duration::from_millis(block_time_ms), produce_empty }
            }
            ProductionTriggerConfig::Open { period_ms } => Self::Open { period: Duration::from_millis(period_ms) },
            ProductionTriggerConfig::Never => Self::Never,
        };
        validate_trigger(trigger)
    }
}

/// Maximum transaction count and encoded bytes in one candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchLimits {
    pub max_transactions: usize,
    pub max_bytes: u64,
}

impl BatchLimits {
    fn validate(self) -> Result<Self, ProducerError> {
        if self.max_transactions == 0 {
            return Err(ProducerError::InvalidConfig("max_transactions must be greater than zero".to_owned()));
        }
        if self.max_bytes == 0 {
            return Err(ProducerError::InvalidConfig("max_bytes must be greater than zero".to_owned()));
        }
        Ok(self)
    }
}

/// Validated producer configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProducerConfig {
    pub trigger: ProductionTrigger,
    pub limits: BatchLimits,
}

impl ProducerConfig {
    /// Validate non-zero limits and policy periods.
    pub fn validate(self) -> Result<Self, ProducerError> {
        self.limits.validate()?;
        validate_trigger(self.trigger)?;
        Ok(self)
    }
}

fn validate_trigger(trigger: ProductionTrigger) -> Result<ProductionTrigger, ProducerError> {
    match trigger {
        ProductionTrigger::Interval { block_time, .. } if block_time.is_zero() => {
            Err(ProducerError::InvalidConfig("interval block_time must be greater than zero".to_owned()))
        }
        ProductionTrigger::Open { period } if period.is_zero() => {
            Err(ProducerError::InvalidConfig("open period must be greater than zero".to_owned()))
        }
        _ => Ok(trigger),
    }
}

/// Reason that closed one candidate window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductionCause {
    Instant,
    Interval,
    Open,
    Manual,
}

/// Reusable trigger state for synchronous hosts.
///
/// `now` is a caller-owned monotonic duration, normally process uptime. The
/// scheduler only decides when a non-finality candidate window closes; it does
/// not inspect application payloads or advance session state.
#[derive(Clone, Debug)]
pub struct ProductionSchedule {
    trigger: ProductionTrigger,
    interval_deadline: Option<Duration>,
    open_deadline: Option<Duration>,
}

impl ProductionSchedule {
    pub fn new(trigger: ProductionTrigger) -> Result<Self, ProducerError> {
        Ok(Self { trigger: validate_trigger(trigger)?, interval_deadline: None, open_deadline: None })
    }

    pub fn trigger(&self) -> ProductionTrigger {
        self.trigger
    }

    /// Observe source availability and return a production cause when the
    /// configured window closes. `limit_reached` is application-neutral: the
    /// source decides whether its count or byte bound is full.
    pub fn observe(&mut self, now: Duration, available: bool, limit_reached: bool) -> Option<ProductionCause> {
        match self.trigger {
            ProductionTrigger::Instant => available.then_some(ProductionCause::Instant),
            ProductionTrigger::Interval { block_time, produce_empty } => {
                let deadline = self.interval_deadline.get_or_insert_with(|| now.saturating_add(block_time));
                if now < *deadline {
                    return None;
                }
                *deadline = now.saturating_add(block_time);
                (available || produce_empty).then_some(ProductionCause::Interval)
            }
            ProductionTrigger::Open { period } => {
                if !available {
                    self.open_deadline = None;
                    return None;
                }
                if limit_reached {
                    self.open_deadline = None;
                    return Some(ProductionCause::Open);
                }
                let deadline = self.open_deadline.get_or_insert_with(|| now.saturating_add(period));
                if now < *deadline {
                    return None;
                }
                self.open_deadline = None;
                Some(ProductionCause::Open)
            }
            ProductionTrigger::Never => None,
        }
    }
}

/// Fixed transaction batch handed to the session-owned commit boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionCandidate {
    pub transactions: Vec<Vec<u8>>,
    pub timestamp_ms: u64,
    pub cause: ProductionCause,
}

impl ProductionCandidate {
    /// Total encoded transaction bytes, checked for overflow.
    pub fn total_bytes(&self) -> Result<u64, ProducerError> {
        transaction_bytes(&self.transactions)
    }
}

/// Durable identity returned only after the candidate has passed finality and
/// the session head has advanced atomically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionReceipt {
    pub block_height: u64,
    pub block_hash: [u8; 32],
}

/// Port-local failure with no claim about session or consensus semantics.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PortError {
    pub message: String,
}

impl PortError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

/// One source reservation. Reserved transactions must remain recoverable until
/// the producer acknowledges a durably committed candidate.
#[derive(Debug)]
pub struct TransactionSelection<R> {
    pub transactions: Vec<Vec<u8>>,
    pub reservation: Option<R>,
}

/// Race-free, reserving source of canonical encoded transactions.
#[async_trait]
pub trait TransactionSource: Send + Sync + 'static {
    type Reservation: Send + 'static;

    /// Subscribe to a monotonically changing availability revision.
    fn subscribe(&self) -> watch::Receiver<u64>;

    /// Reserve up to the supplied remaining batch capacity.
    async fn reserve(&self, limits: BatchLimits) -> Result<TransactionSelection<Self::Reservation>, PortError>;

    /// Forget reservations after the exact candidate is durably committed.
    async fn acknowledge(&self, reservations: Vec<Self::Reservation>) -> Result<(), PortError>;

    /// Make reservations available again after cancellation or failed commit.
    async fn release(&self, reservations: Vec<Self::Reservation>) -> Result<(), PortError>;
}

/// Session-owned boundary that executes, finalises, verifies and atomically
/// commits one fixed candidate. Implementations must return only after durable
/// head advancement.
#[async_trait]
pub trait CandidateCommitter: Send + Sync + 'static {
    async fn commit_candidate(&self, candidate: ProductionCandidate) -> Result<ProductionReceipt, PortError>;
}

/// Wall clock used only for proposed session timestamps.
pub trait WallClock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// UTC wall clock for production use.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
    }
}

/// Producer failures. Every path fails closed; no error marks a candidate as
/// finalised unless a receipt was already returned by the commit boundary.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProducerError {
    #[error("invalid producer configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid manual production request: {0}")]
    InvalidManualRequest(String),
    #[error("manual production is unavailable while the open-window policy is active")]
    ManualUnavailableDuringOpen,
    #[error("transaction source failed: {0}")]
    Source(String),
    #[error("transaction source failed ({source_error}); reservation release also failed ({release})")]
    SourceAndRelease { source_error: String, release: String },
    #[error("candidate commit failed: {0}")]
    Commit(String),
    #[error("candidate commit failed ({commit}); reservation release also failed ({release})")]
    CommitAndRelease { commit: String, release: String },
    #[error("block {block_height} ({block_hash:?}) committed, then source acknowledgement failed: {message}")]
    AcknowledgeAfterCommit { block_height: u64, block_hash: [u8; 32], message: String },
    #[error("transaction batch exceeds its configured limit: {0}")]
    BatchLimit(String),
    #[error("block producer command channel is closed")]
    CommandsClosed,
}

enum ProducerCommand {
    ProduceFromSource { blocks: u32, reply: oneshot::Sender<Result<Vec<ProductionReceipt>, ProducerError>> },
    ProduceTransactions { transactions: Vec<Vec<u8>>, reply: oneshot::Sender<Result<ProductionReceipt, ProducerError>> },
    Shutdown { reply: oneshot::Sender<()> },
}

/// Cloneable operator handle. Commands are serialised with automatic
/// production, so two candidates never advance the same head concurrently.
#[derive(Clone)]
pub struct ProducerHandle {
    commands: mpsc::Sender<ProducerCommand>,
    trigger: ProductionTrigger,
}

impl ProducerHandle {
    /// Produce one or more bounded blocks from the configured source.
    pub async fn produce_blocks(&self, blocks: u32) -> Result<Vec<ProductionReceipt>, ProducerError> {
        self.ensure_manual_available()?;
        if blocks == 0 {
            return Err(ProducerError::InvalidManualRequest("blocks must be greater than zero".to_owned()));
        }
        let (reply, response) = oneshot::channel();
        self.commands.send(ProducerCommand::ProduceFromSource { blocks, reply }).await.map_err(|_| ProducerError::CommandsClosed)?;
        response.await.map_err(|_| ProducerError::CommandsClosed)?
    }

    /// Produce one block from an explicit, already ordered transaction batch.
    pub async fn produce_block_with_transactions(&self, transactions: Vec<Vec<u8>>) -> Result<ProductionReceipt, ProducerError> {
        self.ensure_manual_available()?;
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ProducerCommand::ProduceTransactions { transactions, reply })
            .await
            .map_err(|_| ProducerError::CommandsClosed)?;
        response.await.map_err(|_| ProducerError::CommandsClosed)?
    }

    /// Request orderly shutdown. An open window releases every reservation.
    pub async fn shutdown(&self) -> Result<(), ProducerError> {
        let (reply, response) = oneshot::channel();
        self.commands.send(ProducerCommand::Shutdown { reply }).await.map_err(|_| ProducerError::CommandsClosed)?;
        response.await.map_err(|_| ProducerError::CommandsClosed)
    }

    fn ensure_manual_available(&self) -> Result<(), ProducerError> {
        if matches!(self.trigger, ProductionTrigger::Open { .. }) {
            return Err(ProducerError::ManualUnavailableDuringOpen);
        }
        Ok(())
    }
}

/// Single-writer producer service.
pub struct BlockProducer<S: TransactionSource, C: CandidateCommitter, W: WallClock = SystemWallClock> {
    config: ProducerConfig,
    source: Arc<S>,
    committer: Arc<C>,
    clock: Arc<W>,
    commands: mpsc::Receiver<ProducerCommand>,
}

impl<S: TransactionSource, C: CandidateCommitter> BlockProducer<S, C, SystemWallClock> {
    /// Create a service and operator handle using the system UTC clock.
    pub fn new(config: ProducerConfig, source: Arc<S>, committer: Arc<C>) -> Result<(Self, ProducerHandle), ProducerError> {
        Self::with_clock(config, source, committer, Arc::new(SystemWallClock))
    }
}

impl<S: TransactionSource, C: CandidateCommitter, W: WallClock> BlockProducer<S, C, W> {
    /// Create a service with an injected wall clock for deterministic hosts and tests.
    pub fn with_clock(
        config: ProducerConfig,
        source: Arc<S>,
        committer: Arc<C>,
        clock: Arc<W>,
    ) -> Result<(Self, ProducerHandle), ProducerError> {
        let config = config.validate()?;
        let (commands, receiver) = mpsc::channel(32);
        Ok((Self { config, source, committer, clock, commands: receiver }, ProducerHandle { commands, trigger: config.trigger }))
    }

    /// Run until orderly shutdown or the first source/commit failure.
    pub async fn run(mut self) -> Result<(), ProducerError> {
        match self.config.trigger {
            ProductionTrigger::Instant => self.run_instant().await,
            ProductionTrigger::Interval { block_time, produce_empty } => self.run_interval(block_time, produce_empty).await,
            ProductionTrigger::Open { period } => self.run_open(period).await,
            ProductionTrigger::Never => self.run_never().await,
        }
    }

    async fn run_instant(&mut self) -> Result<(), ProducerError> {
        let mut availability = self.source.subscribe();
        loop {
            let selection = self.reserve(self.config.limits).await?;
            if !selection.transactions.is_empty() {
                self.commit_selection(selection, ProductionCause::Instant).await?;
                continue;
            }
            tokio::select! {
                changed = availability.changed() => {
                    changed.map_err(|_| ProducerError::Source("availability subscription closed".to_owned()))?;
                }
                command = self.commands.recv() => {
                    if self.handle_command(command).await? {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn run_interval(&mut self, block_time: Duration, produce_empty: bool) -> Result<(), ProducerError> {
        let mut next_tick = tokio::time::Instant::now() + block_time;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(next_tick) => {
                    let selection = self.reserve(self.config.limits).await?;
                    if produce_empty || !selection.transactions.is_empty() {
                        self.commit_selection(selection, ProductionCause::Interval).await?;
                    }
                    next_tick += block_time;
                    let now = tokio::time::Instant::now();
                    if next_tick < now {
                        next_tick = now + block_time;
                    }
                }
                command = self.commands.recv() => {
                    if self.handle_command(command).await? {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn run_open(&mut self, period: Duration) -> Result<(), ProducerError> {
        let mut availability = self.source.subscribe();
        loop {
            let first = loop {
                let selection = self.reserve(self.config.limits).await?;
                if !selection.transactions.is_empty() {
                    break selection;
                }
                tokio::select! {
                    changed = availability.changed() => {
                        changed.map_err(|_| ProducerError::Source("availability subscription closed".to_owned()))?;
                    }
                    command = self.commands.recv() => {
                        match command {
                            Some(ProducerCommand::Shutdown { reply }) => {
                                let _ = reply.send(());
                                return Ok(());
                            }
                            Some(ProducerCommand::ProduceFromSource { reply, .. }) => {
                                let _ = reply.send(Err(ProducerError::ManualUnavailableDuringOpen));
                            }
                            Some(ProducerCommand::ProduceTransactions { reply, .. }) => {
                                let _ = reply.send(Err(ProducerError::ManualUnavailableDuringOpen));
                            }
                            None => return Ok(()),
                        }
                    }
                }
            };
            let deadline = tokio::time::Instant::now() + period;
            let mut transactions = first.transactions;
            let mut reservations = first.reservation.into_iter().collect::<Vec<_>>();
            let mut total_bytes = transaction_bytes(&transactions)?;

            loop {
                let remaining = remaining_limits(self.config.limits, transactions.len(), total_bytes)?;
                if remaining.max_transactions == 0 || remaining.max_bytes == 0 {
                    break;
                }
                let selection = match self.reserve(remaining).await {
                    Ok(selection) => selection,
                    Err(error) => {
                        self.release(reservations).await?;
                        return Err(error);
                    }
                };
                if let Some(reservation) = selection.reservation {
                    reservations.push(reservation);
                }
                if !selection.transactions.is_empty() {
                    total_bytes = total_bytes
                        .checked_add(transaction_bytes(&selection.transactions)?)
                        .ok_or_else(|| ProducerError::BatchLimit("transaction bytes overflow".to_owned()))?;
                    transactions.extend(selection.transactions);
                    continue;
                }

                tokio::select! {
                    () = tokio::time::sleep_until(deadline) => break,
                    changed = availability.changed() => {
                        if changed.is_err() {
                            self.release(reservations).await?;
                            return Err(ProducerError::Source("availability subscription closed".to_owned()));
                        }
                    }
                    command = self.commands.recv() => {
                        match command {
                            Some(ProducerCommand::Shutdown { reply }) => {
                                self.release(reservations).await?;
                                let _ = reply.send(());
                                return Ok(());
                            }
                            Some(ProducerCommand::ProduceFromSource { reply, .. }) => {
                                let _ = reply.send(Err(ProducerError::ManualUnavailableDuringOpen));
                            }
                            Some(ProducerCommand::ProduceTransactions { reply, .. }) => {
                                let _ = reply.send(Err(ProducerError::ManualUnavailableDuringOpen));
                            }
                            None => {
                                self.release(reservations).await?;
                                return Ok(());
                            }
                        }
                    }
                }
            }

            self.commit_reserved(transactions, reservations, ProductionCause::Open).await?;
        }
    }

    async fn run_never(&mut self) -> Result<(), ProducerError> {
        loop {
            let command = self.commands.recv().await;
            if self.handle_command(command).await? {
                return Ok(());
            }
        }
    }

    async fn handle_command(&mut self, command: Option<ProducerCommand>) -> Result<bool, ProducerError> {
        let Some(command) = command else {
            return Ok(true);
        };
        match command {
            ProducerCommand::ProduceFromSource { blocks, reply } => {
                let result = self.produce_manual_blocks(blocks).await;
                let fatal = result.as_ref().is_err_and(ProducerError::is_fatal);
                let _ = reply.send(result.clone());
                if fatal {
                    result?;
                }
                Ok(false)
            }
            ProducerCommand::ProduceTransactions { transactions, reply } => {
                let result = self.commit_explicit(transactions).await;
                let fatal = result.as_ref().is_err_and(ProducerError::is_fatal);
                let _ = reply.send(result.clone());
                if fatal {
                    result?;
                }
                Ok(false)
            }
            ProducerCommand::Shutdown { reply } => {
                let _ = reply.send(());
                Ok(true)
            }
        }
    }

    async fn produce_manual_blocks(&self, blocks: u32) -> Result<Vec<ProductionReceipt>, ProducerError> {
        let mut receipts = Vec::with_capacity(blocks as usize);
        for _ in 0..blocks {
            let selection = self.reserve(self.config.limits).await?;
            receipts.push(self.commit_selection(selection, ProductionCause::Manual).await?);
        }
        Ok(receipts)
    }

    async fn commit_explicit(&self, transactions: Vec<Vec<u8>>) -> Result<ProductionReceipt, ProducerError> {
        validate_transactions(&transactions, self.config.limits)?;
        self.commit_reserved(transactions, Vec::new(), ProductionCause::Manual).await
    }

    async fn reserve(&self, limits: BatchLimits) -> Result<TransactionSelection<S::Reservation>, ProducerError> {
        let selection = self.source.reserve(limits).await.map_err(|error| ProducerError::Source(error.to_string()))?;
        match validate_selection(&selection, limits) {
            Ok(()) => Ok(selection),
            Err(error) => {
                let reservations = selection.reservation.into_iter().collect();
                match self.source.release(reservations).await {
                    Ok(()) => Err(error),
                    Err(release) => {
                        Err(ProducerError::SourceAndRelease { source_error: error.to_string(), release: release.to_string() })
                    }
                }
            }
        }
    }

    async fn commit_selection(
        &self,
        selection: TransactionSelection<S::Reservation>,
        cause: ProductionCause,
    ) -> Result<ProductionReceipt, ProducerError> {
        let reservations = selection.reservation.into_iter().collect();
        self.commit_reserved(selection.transactions, reservations, cause).await
    }

    async fn commit_reserved(
        &self,
        transactions: Vec<Vec<u8>>,
        reservations: Vec<S::Reservation>,
        cause: ProductionCause,
    ) -> Result<ProductionReceipt, ProducerError> {
        validate_transactions(&transactions, self.config.limits)?;
        let candidate = ProductionCandidate { transactions, timestamp_ms: self.clock.now_ms(), cause };
        match self.committer.commit_candidate(candidate).await {
            Ok(receipt) => {
                self.source.acknowledge(reservations).await.map_err(|error| ProducerError::AcknowledgeAfterCommit {
                    block_height: receipt.block_height,
                    block_hash: receipt.block_hash,
                    message: error.to_string(),
                })?;
                Ok(receipt)
            }
            Err(commit) => match self.source.release(reservations).await {
                Ok(()) => Err(ProducerError::Commit(commit.to_string())),
                Err(release) => Err(ProducerError::CommitAndRelease { commit: commit.to_string(), release: release.to_string() }),
            },
        }
    }

    async fn release(&self, reservations: Vec<S::Reservation>) -> Result<(), ProducerError> {
        self.source.release(reservations).await.map_err(|error| ProducerError::Source(error.to_string()))
    }
}

impl ProducerError {
    fn is_fatal(&self) -> bool {
        !matches!(self, Self::InvalidManualRequest(_) | Self::ManualUnavailableDuringOpen | Self::BatchLimit(_))
    }
}

fn validate_selection<R>(selection: &TransactionSelection<R>, limits: BatchLimits) -> Result<(), ProducerError> {
    if selection.transactions.is_empty() && selection.reservation.is_some() {
        return Err(ProducerError::Source("source returned an empty reservation".to_owned()));
    }
    if !selection.transactions.is_empty() && selection.reservation.is_none() {
        return Err(ProducerError::Source("source returned transactions without a reservation".to_owned()));
    }
    validate_transactions(&selection.transactions, limits)
        .map_err(|error| ProducerError::Source(format!("source returned an invalid selection: {error}")))
}

fn validate_transactions(transactions: &[Vec<u8>], limits: BatchLimits) -> Result<(), ProducerError> {
    if transactions.len() > limits.max_transactions {
        return Err(ProducerError::BatchLimit(format!(
            "transaction count {} exceeds {}",
            transactions.len(),
            limits.max_transactions
        )));
    }
    let bytes = transaction_bytes(transactions)?;
    if bytes > limits.max_bytes {
        return Err(ProducerError::BatchLimit(format!("transaction bytes {bytes} exceed {}", limits.max_bytes)));
    }
    Ok(())
}

fn transaction_bytes(transactions: &[Vec<u8>]) -> Result<u64, ProducerError> {
    transactions.iter().try_fold(0u64, |total, transaction| {
        total.checked_add(transaction.len() as u64).ok_or_else(|| ProducerError::BatchLimit("transaction bytes overflow".to_owned()))
    })
}

fn remaining_limits(limits: BatchLimits, count: usize, bytes: u64) -> Result<BatchLimits, ProducerError> {
    Ok(BatchLimits {
        max_transactions: limits
            .max_transactions
            .checked_sub(count)
            .ok_or_else(|| ProducerError::BatchLimit("transaction count overflow".to_owned()))?,
        max_bytes: limits
            .max_bytes
            .checked_sub(bytes)
            .ok_or_else(|| ProducerError::BatchLimit("transaction bytes overflow".to_owned()))?,
    })
}

#[derive(Default)]
struct QueueState {
    available: VecDeque<Vec<u8>>,
    reserved: BTreeMap<u64, Vec<Vec<u8>>>,
    next_reservation: u64,
}

/// Small FIFO source useful for embedding applications and deterministic tests.
/// Production mempool adapters can implement [`TransactionSource`] directly.
pub struct QueuedTransactionSource {
    state: tokio::sync::Mutex<QueueState>,
    revision: watch::Sender<u64>,
    max_transactions: usize,
    max_transaction_bytes: u64,
}

impl QueuedTransactionSource {
    pub fn new(max_transactions: usize, max_transaction_bytes: u64) -> Result<Self, ProducerError> {
        if max_transactions == 0 || max_transaction_bytes == 0 {
            return Err(ProducerError::InvalidConfig(
                "queue transaction capacity and max_transaction_bytes must be greater than zero".to_owned(),
            ));
        }
        let (revision, _) = watch::channel(0);
        Ok(Self { state: tokio::sync::Mutex::new(QueueState::default()), revision, max_transactions, max_transaction_bytes })
    }

    /// Submit canonical encoded transaction bytes and notify every producer subscriber.
    pub async fn submit(&self, transaction: Vec<u8>) -> Result<(), PortError> {
        if transaction.len() as u64 > self.max_transaction_bytes {
            return Err(PortError::new(format!(
                "transaction bytes {} exceed queue limit {}",
                transaction.len(),
                self.max_transaction_bytes
            )));
        }
        let mut state = self.state.lock().await;
        let queued = state.available.len() + state.reserved.values().map(Vec::len).sum::<usize>();
        if queued >= self.max_transactions {
            return Err(PortError::new(format!("transaction queue is full at {} entries", self.max_transactions)));
        }
        state.available.push_back(transaction);
        drop(state);
        let next = self.revision.borrow().wrapping_add(1);
        self.revision.send_replace(next);
        Ok(())
    }

    /// Number of available and reserved transactions.
    pub async fn len(&self) -> usize {
        let state = self.state.lock().await;
        state.available.len() + state.reserved.values().map(Vec::len).sum::<usize>()
    }

    /// Whether the queue contains no available or reserved transactions.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[async_trait]
impl TransactionSource for QueuedTransactionSource {
    type Reservation = u64;

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    async fn reserve(&self, limits: BatchLimits) -> Result<TransactionSelection<Self::Reservation>, PortError> {
        let mut state = self.state.lock().await;
        let mut transactions = Vec::new();
        let mut bytes = 0u64;
        while transactions.len() < limits.max_transactions {
            let Some(next) = state.available.front() else {
                break;
            };
            let next_bytes = next.len() as u64;
            if next_bytes > limits.max_bytes && transactions.is_empty() {
                return Err(PortError::new(format!(
                    "front transaction bytes {next_bytes} exceed batch capacity {}",
                    limits.max_bytes
                )));
            }
            if bytes.saturating_add(next_bytes) > limits.max_bytes {
                break;
            }
            bytes += next_bytes;
            transactions.push(state.available.pop_front().expect("front transaction exists"));
        }
        if transactions.is_empty() {
            return Ok(TransactionSelection { transactions, reservation: None });
        }
        let reservation = state.next_reservation;
        state.next_reservation = state.next_reservation.checked_add(1).ok_or_else(|| PortError::new("reservation id overflow"))?;
        state.reserved.insert(reservation, transactions.clone());
        Ok(TransactionSelection { transactions, reservation: Some(reservation) })
    }

    async fn acknowledge(&self, reservations: Vec<Self::Reservation>) -> Result<(), PortError> {
        let mut state = self.state.lock().await;
        if reservations.iter().copied().collect::<std::collections::BTreeSet<_>>().len() != reservations.len() {
            return Err(PortError::new("duplicate reservation"));
        }
        if reservations.iter().any(|reservation| !state.reserved.contains_key(reservation)) {
            return Err(PortError::new("one or more reservations are unknown"));
        }
        for reservation in reservations {
            state.reserved.remove(&reservation);
        }
        Ok(())
    }

    async fn release(&self, reservations: Vec<Self::Reservation>) -> Result<(), PortError> {
        if reservations.is_empty() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        if reservations.iter().copied().collect::<std::collections::BTreeSet<_>>().len() != reservations.len() {
            return Err(PortError::new("duplicate reservation"));
        }
        if reservations.iter().any(|reservation| !state.reserved.contains_key(reservation)) {
            return Err(PortError::new("one or more reservations are unknown"));
        }
        let mut released = Vec::new();
        for reservation in reservations {
            let mut transactions = state.reserved.remove(&reservation).expect("reservation presence was checked");
            released.append(&mut transactions);
        }
        for transaction in released.into_iter().rev() {
            state.available.push_front(transaction);
        }
        drop(state);
        let next = self.revision.borrow().wrapping_add(1);
        self.revision.send_replace(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::Mutex as AsyncMutex;

    #[derive(Default)]
    struct TestClock(AtomicU64);

    impl WallClock for TestClock {
        fn now_ms(&self) -> u64 {
            self.0.fetch_add(1, Ordering::SeqCst)
        }
    }

    struct RecordingCommitter {
        candidates: mpsc::UnboundedSender<ProductionCandidate>,
        next_height: AtomicU64,
        fail: AsyncMutex<Option<String>>,
    }

    impl RecordingCommitter {
        fn new() -> (Self, mpsc::UnboundedReceiver<ProductionCandidate>) {
            let (candidates, receiver) = mpsc::unbounded_channel();
            (Self { candidates, next_height: AtomicU64::new(0), fail: AsyncMutex::new(None) }, receiver)
        }
    }

    #[async_trait]
    impl CandidateCommitter for RecordingCommitter {
        async fn commit_candidate(&self, candidate: ProductionCandidate) -> Result<ProductionReceipt, PortError> {
            if let Some(message) = self.fail.lock().await.take() {
                return Err(PortError::new(message));
            }
            self.candidates.send(candidate).map_err(|_| PortError::new("candidate receiver closed"))?;
            let block_height = self.next_height.fetch_add(1, Ordering::SeqCst);
            Ok(ProductionReceipt { block_height, block_hash: [block_height as u8; 32] })
        }
    }

    fn config(trigger: ProductionTrigger) -> ProducerConfig {
        ProducerConfig { trigger, limits: BatchLimits { max_transactions: 3, max_bytes: 8 } }
    }

    async fn service(
        trigger: ProductionTrigger,
    ) -> (
        Arc<QueuedTransactionSource>,
        Arc<RecordingCommitter>,
        mpsc::UnboundedReceiver<ProductionCandidate>,
        ProducerHandle,
        tokio::task::JoinHandle<Result<(), ProducerError>>,
    ) {
        let source = Arc::new(QueuedTransactionSource::new(16, 8).unwrap());
        let (committer, candidates) = RecordingCommitter::new();
        let committer = Arc::new(committer);
        let (producer, handle) =
            BlockProducer::with_clock(config(trigger), Arc::clone(&source), Arc::clone(&committer), Arc::new(TestClock::default()))
                .unwrap();
        let task = tokio::spawn(producer.run());
        (source, committer, candidates, handle, task)
    }

    #[test]
    fn configuration_rejects_zero_limits_and_periods() {
        assert!(matches!(
            ProducerConfig { trigger: ProductionTrigger::Never, limits: BatchLimits { max_transactions: 0, max_bytes: 1 } }.validate(),
            Err(ProducerError::InvalidConfig(_))
        ));
        assert!(matches!(
            config(ProductionTrigger::Interval { block_time: Duration::ZERO, produce_empty: false }).validate(),
            Err(ProducerError::InvalidConfig(_))
        ));
        assert!(matches!(config(ProductionTrigger::Open { period: Duration::ZERO }).validate(), Err(ProducerError::InvalidConfig(_))));
    }

    #[test]
    fn trigger_config_is_strict_and_application_neutral() {
        let config: ProductionTriggerConfig = serde_json::from_str(r#"{"type":"open","period_ms":100}"#).unwrap();
        assert_eq!(ProductionTrigger::try_from(config).unwrap(), ProductionTrigger::Open { period: Duration::from_millis(100) });
        assert!(serde_json::from_str::<ProductionTriggerConfig>(r#"{"type":"open","period_ms":100,"veloren_ticks":3}"#).is_err());
        assert!(matches!(
            ProductionTrigger::try_from(ProductionTriggerConfig::Open { period_ms: 0 }),
            Err(ProducerError::InvalidConfig(_))
        ));
    }

    #[test]
    fn reusable_schedule_opens_only_after_availability() {
        let mut schedule = ProductionSchedule::new(ProductionTrigger::Open { period: Duration::from_millis(100) }).unwrap();
        assert_eq!(schedule.observe(Duration::ZERO, false, false), None);
        assert_eq!(schedule.observe(Duration::from_secs(10), false, false), None);
        assert_eq!(schedule.observe(Duration::from_secs(10), true, false), None);
        assert_eq!(schedule.observe(Duration::from_millis(10_099), true, false), None);
        assert_eq!(schedule.observe(Duration::from_millis(10_100), true, false), Some(ProductionCause::Open));
        assert_eq!(schedule.observe(Duration::from_millis(10_101), true, true), Some(ProductionCause::Open));
    }

    #[tokio::test]
    async fn instant_closes_bounded_batches_when_transactions_arrive() {
        let (source, _, mut candidates, handle, task) = service(ProductionTrigger::Instant).await;
        source.submit(vec![1, 2, 3]).await.unwrap();
        source.submit(vec![4, 5, 6]).await.unwrap();
        source.submit(vec![7, 8, 9]).await.unwrap();

        let first = tokio::time::timeout(Duration::from_secs(1), candidates.recv()).await.unwrap().unwrap();
        let second = tokio::time::timeout(Duration::from_secs(1), candidates.recv()).await.unwrap().unwrap();
        assert_eq!(first.cause, ProductionCause::Instant);
        assert_eq!(first.transactions, vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(second.transactions, vec![vec![7, 8, 9]]);
        assert!(source.is_empty().await);

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn interval_uses_fixed_ticks_and_can_commit_empty_batches() {
        let (_, _, mut candidates, handle, task) =
            service(ProductionTrigger::Interval { block_time: Duration::from_secs(5), produce_empty: true }).await;
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());

        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        let candidate = candidates.recv().await.unwrap();
        assert_eq!(candidate.cause, ProductionCause::Interval);
        assert!(candidate.transactions.is_empty());

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn interval_skips_empty_batches_unless_explicitly_enabled() {
        let (source, _, mut candidates, handle, task) =
            service(ProductionTrigger::Interval { block_time: Duration::from_secs(5), produce_empty: false }).await;
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());

        source.submit(vec![1, 2, 3]).await.unwrap();
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert_eq!(candidates.recv().await.unwrap().transactions, vec![vec![1, 2, 3]]);

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn open_does_not_create_periodic_empty_candidates() {
        let (source, _, mut candidates, handle, task) = service(ProductionTrigger::Open { period: Duration::from_secs(10) }).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3_600)).await;
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());

        source.submit(vec![1, 2, 3]).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(candidates.recv().await.unwrap().transactions, vec![vec![1, 2, 3]]);

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn open_accumulates_arrivals_until_its_deadline() {
        let (source, _, mut candidates, handle, task) = service(ProductionTrigger::Open { period: Duration::from_secs(10) }).await;
        source.submit(vec![1, 2]).await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        source.submit(vec![3, 4, 5]).await.unwrap();
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());

        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::task::yield_now().await;
        let candidate = candidates.recv().await.unwrap();
        assert_eq!(candidate.cause, ProductionCause::Open);
        assert_eq!(candidate.transactions, vec![vec![1, 2], vec![3, 4, 5]]);
        assert!(source.is_empty().await);
        assert_eq!(handle.produce_blocks(1).await, Err(ProducerError::ManualUnavailableDuringOpen));

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn open_shutdown_releases_its_in_flight_reservations() {
        let (source, _, _, handle, task) = service(ProductionTrigger::Open { period: Duration::from_secs(10) }).await;
        source.submit(vec![1, 2, 3]).await.unwrap();
        tokio::task::yield_now().await;

        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();

        let selection = source.reserve(config(ProductionTrigger::Never).limits).await.unwrap();
        assert_eq!(selection.transactions, vec![vec![1, 2, 3]]);
        source.release(selection.reservation.into_iter().collect()).await.unwrap();
    }

    #[tokio::test]
    async fn never_produces_only_through_manual_requests() {
        let (source, _, mut candidates, handle, task) = service(ProductionTrigger::Never).await;
        source.submit(vec![1, 2, 3]).await.unwrap();
        tokio::task::yield_now().await;
        assert!(candidates.try_recv().is_err());

        let receipts = handle.produce_blocks(2).await.unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(candidates.recv().await.unwrap().transactions, vec![vec![1, 2, 3]]);
        assert!(candidates.recv().await.unwrap().transactions.is_empty());

        let explicit = vec![vec![9; 4], vec![8; 4]];
        handle.produce_block_with_transactions(explicit.clone()).await.unwrap();
        assert_eq!(candidates.recv().await.unwrap().transactions, explicit);
        assert!(matches!(handle.produce_block_with_transactions(vec![vec![0; 9]]).await, Err(ProducerError::BatchLimit(_))));
        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn failed_commit_releases_reserved_transactions() {
        let source = Arc::new(QueuedTransactionSource::new(8, 8).unwrap());
        source.submit(vec![1, 2, 3]).await.unwrap();
        let (failing, _) = RecordingCommitter::new();
        *failing.fail.lock().await = Some("finality unavailable".to_owned());
        let failing = Arc::new(failing);
        let (producer, handle) =
            BlockProducer::with_clock(config(ProductionTrigger::Never), Arc::clone(&source), failing, Arc::new(TestClock::default()))
                .unwrap();
        let task = tokio::spawn(producer.run());
        assert_eq!(handle.produce_blocks(1).await, Err(ProducerError::Commit("finality unavailable".to_owned())));
        assert_eq!(task.await.unwrap(), Err(ProducerError::Commit("finality unavailable".to_owned())));
        assert_eq!(source.len().await, 1);

        let (working, mut candidates) = RecordingCommitter::new();
        let (producer, handle) = BlockProducer::with_clock(
            config(ProductionTrigger::Never),
            Arc::clone(&source),
            Arc::new(working),
            Arc::new(TestClock::default()),
        )
        .unwrap();
        let task = tokio::spawn(producer.run());
        handle.produce_blocks(1).await.unwrap();
        assert_eq!(candidates.recv().await.unwrap().transactions, vec![vec![1, 2, 3]]);
        handle.shutdown().await.unwrap();
        task.await.unwrap().unwrap();
    }
}
