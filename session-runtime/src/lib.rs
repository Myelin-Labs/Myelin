// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Embeddable composition root for continuous Myelin sessions.
//!
//! This crate contains integration adapters and service lifecycle policy. It
//! is not a daemon and owns no consensus, execution, storage, or CKB evidence
//! business rules.

use async_trait::async_trait;
use futures::FutureExt;
use myelin_consensus::{
    ConsensusCatalog, ConsensusConfig, ConsensusError, ConsensusModuleDescriptor, EncodedConsensusMessage, FinalityProof, MyelinBlock,
    SelectedConsensus,
};
use myelin_session::{
    FinalityVerifier, Hash32, SessionChain, SessionConfig, SessionError, SessionStore, TransitionExecutor, VerifiedFinality,
};
use myelin_session_network::{MessageClass, MessageType, NetworkBinding};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::Duration,
};

/// Top-level adapter from the registered consensus catalog to the
/// session-owned deterministic finality port.
#[derive(Clone, Debug)]
pub struct RegisteredFinalityVerifier {
    consensus: SelectedConsensus,
}

impl RegisteredFinalityVerifier {
    /// Build one exact compiled-in module from strict consensus config.
    pub fn new(config: ConsensusConfig) -> Result<Self, RuntimeError> {
        Ok(Self { consensus: ConsensusCatalog::build(config)? })
    }

    /// Exact selected engine for an in-process trusted driver. Session commit
    /// code still accepts only the verifier port.
    pub fn selected_consensus(&self) -> &SelectedConsensus {
        &self.consensus
    }
}

impl FinalityVerifier for RegisteredFinalityVerifier {
    fn descriptor(&self) -> ConsensusModuleDescriptor {
        self.consensus.module_descriptor()
    }

    fn verify(&self, block: &MyelinBlock, proof: &FinalityProof) -> Result<VerifiedFinality, ConsensusError> {
        let finalised = self.consensus.finalise_with_proof(block.clone(), proof.clone())?;
        Ok(VerifiedFinality { block_hash: finalised.block_hash, consensus_module_commitment: self.consensus.module_commitment() })
    }
}

/// Three identities that must be copied into a session config before genesis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsensusBinding {
    pub consensus_kind: myelin_consensus::ConsensusKind,
    pub consensus_config_commitment: Hash32,
    pub consensus_module_commitment: Hash32,
    pub consensus_wal_schema_hash: Hash32,
}

impl ConsensusBinding {
    /// Derive an immutable binding from one validated registered config.
    pub fn from_config(config: ConsensusConfig) -> Result<Self, RuntimeError> {
        let verifier = RegisteredFinalityVerifier::new(config)?;
        let descriptor = verifier.descriptor();
        Ok(Self {
            consensus_kind: descriptor.consensus_kind,
            consensus_config_commitment: descriptor.config_commitment,
            consensus_module_commitment: descriptor.commitment(),
            consensus_wal_schema_hash: descriptor.wal_schema_hash,
        })
    }

    /// Check that a session config was bound explicitly before persistence.
    pub fn verify_session_config(&self, config: &SessionConfig) -> Result<(), RuntimeError> {
        if config.consensus_kind != self.consensus_kind
            || config.consensus_config_commitment != self.consensus_config_commitment
            || config.consensus_module_commitment != self.consensus_module_commitment
            || config.consensus_wal_schema_hash != self.consensus_wal_schema_hash
        {
            return Err(RuntimeError::Binding("session config does not match the selected registered consensus module".to_owned()));
        }
        Ok(())
    }

    /// Build the transport binding for this exact genesis-bound module.
    pub const fn network_binding(&self, session_id: Hash32) -> NetworkBinding {
        NetworkBinding { session_id, consensus_module_commitment: self.consensus_module_commitment }
    }
}

/// Adapt an owner-encoded consensus frame to the module-neutral transport
/// namespace. The transport never interprets `type_tag` or `payload`.
pub const fn consensus_message_type(message: &EncodedConsensusMessage) -> MessageType {
    MessageType::new(MessageClass::Consensus, message.format_version, message.type_tag)
}

/// Service importance used by health enforcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceCriticality {
    /// Failure stops every service and prevents further writes.
    Critical,
    /// Failure is reported for an embedding application to restart.
    Restartable,
    /// Failure is reported but does not stop critical services.
    Optional,
}

/// Runtime-independent service health.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceHealth {
    Starting,
    Ready,
    Degraded(String),
    Stopped,
    Failed(String),
}

/// One health result returned to operators/embedding applications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceHealthReport {
    pub name: String,
    pub criticality: ServiceCriticality,
    pub health: ServiceHealth,
}

/// Small host-owned lifecycle contract. Domain ports do not expose channels,
/// Tokio handles, or process-management details.
#[async_trait]
pub trait ManagedService: Send + Sync + 'static {
    async fn start(&self) -> Result<(), ManagedServiceError>;
    async fn await_ready(&self) -> Result<(), ManagedServiceError>;
    async fn stop(&self) -> Result<(), ManagedServiceError>;
    fn health(&self) -> ServiceHealth;
}

/// Structured service-local lifecycle failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ManagedServiceError {
    pub message: String,
}

impl ManagedServiceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

struct RegisteredService {
    name: String,
    dependencies: Vec<String>,
    criticality: ServiceCriticality,
    service: Arc<dyn ManagedService>,
}

/// Explicit dependency-ordered, panic-contained service supervisor.
pub struct ServiceSupervisor {
    registrations: Vec<RegisteredService>,
    started: Vec<usize>,
    startup_failures: HashMap<usize, String>,
    lifecycle_timeout: Duration,
}

impl ServiceSupervisor {
    pub fn new(lifecycle_timeout: Duration) -> Result<Self, RuntimeError> {
        if lifecycle_timeout.is_zero() {
            return Err(RuntimeError::Topology("lifecycle timeout must not be zero".to_owned()));
        }
        Ok(Self { registrations: Vec::new(), started: Vec::new(), startup_failures: HashMap::new(), lifecycle_timeout })
    }

    /// Register a service and its explicit readiness dependencies.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        dependencies: Vec<String>,
        criticality: ServiceCriticality,
        service: Arc<dyn ManagedService>,
    ) -> Result<(), RuntimeError> {
        let name = name.into();
        if name.is_empty() || self.registrations.iter().any(|registered| registered.name == name) {
            return Err(RuntimeError::Topology("service names must be non-empty and unique".to_owned()));
        }
        if dependencies.iter().any(|dependency| dependency.is_empty() || dependency == &name) {
            return Err(RuntimeError::Topology(format!("service {name} has an invalid dependency")));
        }
        self.registrations.push(RegisteredService { name, dependencies, criticality, service });
        Ok(())
    }

    /// Start and await readiness in a deterministic topological order.
    pub async fn start_all(&mut self) -> Result<Vec<String>, RuntimeError> {
        if !self.started.is_empty() {
            return Err(RuntimeError::Topology("services are already started".to_owned()));
        }
        self.startup_failures.clear();
        let order = self.topological_order()?;
        for index in order {
            let dependencies_ready = self.registrations[index].dependencies.iter().all(|dependency| {
                self.registrations
                    .iter()
                    .position(|registered| &registered.name == dependency)
                    .is_some_and(|dependency_index| self.started.contains(&dependency_index))
            });
            if !dependencies_ready {
                let error = RuntimeError::Service {
                    name: self.registrations[index].name.clone(),
                    phase: "dependency-readiness",
                    message: "one or more dependencies did not become ready".to_owned(),
                };
                if self.registrations[index].criticality == ServiceCriticality::Critical {
                    self.stop_started_best_effort().await;
                    return Err(error);
                }
                self.startup_failures.insert(index, error.to_string());
                continue;
            }

            let name = self.registrations[index].name.clone();
            let criticality = self.registrations[index].criticality;
            let service = Arc::clone(&self.registrations[index].service);
            if let Err(error) = run_phase(&name, "start", self.lifecycle_timeout, service.start()).await {
                let _ = run_phase(&name, "stop-after-start-failure", self.lifecycle_timeout, service.stop()).await;
                if criticality == ServiceCriticality::Critical {
                    self.stop_started_best_effort().await;
                    return Err(error);
                }
                self.startup_failures.insert(index, error.to_string());
                continue;
            }

            self.started.push(index);
            if let Err(error) = run_phase(&name, "ready", self.lifecycle_timeout, service.await_ready()).await {
                if criticality == ServiceCriticality::Critical {
                    self.stop_started_best_effort().await;
                    return Err(error);
                }
                self.started.pop();
                let _ = run_phase(&name, "stop-after-readiness-failure", self.lifecycle_timeout, service.stop()).await;
                self.startup_failures.insert(index, error.to_string());
            }
        }
        Ok(self.started.iter().map(|index| self.registrations[*index].name.clone()).collect())
    }

    /// Stop in reverse dependency order with bounded, panic-contained calls.
    pub async fn stop_all(&mut self) -> Result<Vec<String>, RuntimeError> {
        let mut stopped = Vec::new();
        let mut first_error = None;
        while let Some(index) = self.started.pop() {
            let registered = &self.registrations[index];
            match run_phase(&registered.name, "stop", self.lifecycle_timeout, registered.service.stop()).await {
                Ok(()) => stopped.push(registered.name.clone()),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => {
                self.startup_failures.clear();
                Err(error)
            }
            None => {
                self.startup_failures.clear();
                Ok(stopped)
            }
        }
    }

    /// Snapshot health and fail closed if a critical service is not ready.
    pub async fn enforce_health(&mut self) -> Result<Vec<ServiceHealthReport>, RuntimeError> {
        let reports = self.health_reports();
        let failed_critical = reports
            .iter()
            .find(|report| report.criticality == ServiceCriticality::Critical && !matches!(report.health, ServiceHealth::Ready));
        if let Some(failed) = failed_critical {
            let name = failed.name.clone();
            let health = failed.health.clone();
            let _ = self.stop_all().await;
            return Err(RuntimeError::CriticalService { name, health });
        }
        Ok(reports)
    }

    fn health_reports(&self) -> Vec<ServiceHealthReport> {
        self.registrations
            .iter()
            .enumerate()
            .map(|(index, registered)| ServiceHealthReport {
                name: registered.name.clone(),
                criticality: registered.criticality,
                health: self.startup_failures.get(&index).map_or_else(
                    || {
                        std::panic::catch_unwind(AssertUnwindSafe(|| registered.service.health()))
                            .unwrap_or_else(|_| ServiceHealth::Failed("health probe panicked".to_owned()))
                    },
                    |failure| ServiceHealth::Failed(failure.clone()),
                ),
            })
            .collect()
    }

    fn critical_health_failure(&self) -> Option<(String, ServiceHealth)> {
        self.health_reports()
            .into_iter()
            .find(|report| report.criticality == ServiceCriticality::Critical && !matches!(report.health, ServiceHealth::Ready))
            .map(|report| (report.name, report.health))
    }

    fn topological_order(&self) -> Result<Vec<usize>, RuntimeError> {
        let indexes =
            self.registrations.iter().enumerate().map(|(index, service)| (service.name.as_str(), index)).collect::<HashMap<_, _>>();
        let mut indegree = vec![0_usize; self.registrations.len()];
        let mut dependents = vec![Vec::new(); self.registrations.len()];
        for (index, service) in self.registrations.iter().enumerate() {
            for dependency in &service.dependencies {
                let Some(dependency_index) = indexes.get(dependency.as_str()).copied() else {
                    return Err(RuntimeError::Topology(format!("service {} depends on unknown service {dependency}", service.name)));
                };
                indegree[index] += 1;
                dependents[dependency_index].push(index);
            }
        }
        let mut ready =
            indegree.iter().enumerate().filter_map(|(index, degree)| (*degree == 0).then_some(index)).collect::<VecDeque<_>>();
        let mut order = Vec::with_capacity(self.registrations.len());
        while let Some(index) = ready.pop_front() {
            order.push(index);
            for dependent in &dependents[index] {
                indegree[*dependent] -= 1;
                if indegree[*dependent] == 0 {
                    ready.push_back(*dependent);
                }
            }
        }
        if order.len() != self.registrations.len() {
            return Err(RuntimeError::Topology("service dependency graph contains a cycle".to_owned()));
        }
        Ok(order)
    }

    async fn stop_started_best_effort(&mut self) {
        while let Some(index) = self.started.pop() {
            let registered = &self.registrations[index];
            let _ = run_phase(&registered.name, "stop-after-failure", self.lifecycle_timeout, registered.service.stop()).await;
        }
    }
}

async fn run_phase<F>(name: &str, phase: &'static str, timeout: Duration, future: F) -> Result<(), RuntimeError>
where
    F: Future<Output = Result<(), ManagedServiceError>> + Send,
{
    let future = AssertUnwindSafe(future).catch_unwind();
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(RuntimeError::Service { name: name.to_owned(), phase, message: error.to_string() }),
        Ok(Err(_)) => Err(RuntimeError::Service { name: name.to_owned(), phase, message: "service panicked".to_owned() }),
        Err(_) => Err(RuntimeError::Service { name: name.to_owned(), phase, message: "lifecycle call timed out".to_owned() }),
    }
}

/// Continuous session plus its explicitly managed host services.
pub struct RuntimeHost<E: TransitionExecutor, S: SessionStore> {
    chain: SessionChain<E, S>,
    supervisor: ServiceSupervisor,
    writer_enabled: bool,
}

impl<E: TransitionExecutor, S: SessionStore> RuntimeHost<E, S> {
    pub fn create(
        session_config: SessionConfig,
        consensus_config: ConsensusConfig,
        executor: E,
        store: Arc<S>,
        supervisor: ServiceSupervisor,
    ) -> Result<Self, RuntimeError> {
        let verifier = Arc::new(RegisteredFinalityVerifier::new(consensus_config)?);
        let descriptor = verifier.descriptor();
        ConsensusBinding {
            consensus_kind: descriptor.consensus_kind,
            consensus_config_commitment: descriptor.config_commitment,
            consensus_module_commitment: descriptor.commitment(),
            consensus_wal_schema_hash: descriptor.wal_schema_hash,
        }
        .verify_session_config(&session_config)?;
        let chain = SessionChain::create(session_config, verifier, executor, store)?;
        Ok(Self { chain, supervisor, writer_enabled: false })
    }

    pub fn recover(
        session_id: Hash32,
        consensus_config: ConsensusConfig,
        executor: E,
        store: Arc<S>,
        supervisor: ServiceSupervisor,
    ) -> Result<Self, RuntimeError> {
        let verifier = Arc::new(RegisteredFinalityVerifier::new(consensus_config)?);
        let chain = SessionChain::recover(session_id, verifier, executor, store)?;
        Ok(Self { chain, supervisor, writer_enabled: false })
    }

    pub fn chain(&self) -> &SessionChain<E, S> {
        &self.chain
    }

    /// Mutable access is available only while all critical services are ready.
    pub fn chain_mut(&mut self) -> Result<&mut SessionChain<E, S>, RuntimeError> {
        if !self.writer_enabled {
            return Err(RuntimeError::WriterDisabled);
        }
        if let Some((name, health)) = self.supervisor.critical_health_failure() {
            self.writer_enabled = false;
            return Err(RuntimeError::CriticalService { name, health });
        }
        Ok(&mut self.chain)
    }

    /// Whether session writes are currently admitted by the host lifecycle.
    pub const fn writer_enabled(&self) -> bool {
        self.writer_enabled
    }

    pub async fn start_services(&mut self) -> Result<Vec<String>, RuntimeError> {
        let started = self.supervisor.start_all().await?;
        if let Err(error) = self.supervisor.enforce_health().await {
            self.writer_enabled = false;
            return Err(error);
        }
        self.writer_enabled = true;
        Ok(started)
    }

    pub async fn stop_services(&mut self) -> Result<Vec<String>, RuntimeError> {
        self.writer_enabled = false;
        self.supervisor.stop_all().await
    }

    pub async fn enforce_health(&mut self) -> Result<Vec<ServiceHealthReport>, RuntimeError> {
        match self.supervisor.enforce_health().await {
            Ok(reports) => Ok(reports),
            Err(error) => {
                self.writer_enabled = false;
                Err(error)
            }
        }
    }
}

/// Composition and lifecycle failures.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Consensus(#[from] ConsensusError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("invalid consensus binding: {0}")]
    Binding(String),
    #[error("invalid service topology: {0}")]
    Topology(String),
    #[error("service {name} failed during {phase}: {message}")]
    Service { name: String, phase: &'static str, message: String },
    #[error("critical service {name} is not ready: {health:?}")]
    CriticalService { name: String, health: ServiceHealth },
    #[error("session writer is disabled until critical runtime services are ready")]
    WriterDisabled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_consensus::{Authority, CommitteeSigner, ProofOfAuthorityConfig};
    use myelin_session::{
        ConsensusWal, ExecutionOutcome, FinalisedBlockRecord, PendingDelivery, SessionGenesis, SessionHead, StoreError,
    };
    use std::sync::Mutex;

    struct FakeService {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        health: Arc<Mutex<ServiceHealth>>,
    }

    #[async_trait]
    impl ManagedService for FakeService {
        async fn start(&self) -> Result<(), ManagedServiceError> {
            self.events.lock().unwrap().push(format!("start:{}", self.name));
            *self.health.lock().unwrap() = ServiceHealth::Starting;
            Ok(())
        }

        async fn await_ready(&self) -> Result<(), ManagedServiceError> {
            self.events.lock().unwrap().push(format!("ready:{}", self.name));
            *self.health.lock().unwrap() = ServiceHealth::Ready;
            Ok(())
        }

        async fn stop(&self) -> Result<(), ManagedServiceError> {
            self.events.lock().unwrap().push(format!("stop:{}", self.name));
            *self.health.lock().unwrap() = ServiceHealth::Stopped;
            Ok(())
        }

        fn health(&self) -> ServiceHealth {
            self.health.lock().unwrap().clone()
        }
    }

    fn fake(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> (Arc<FakeService>, Arc<Mutex<ServiceHealth>>) {
        let health = Arc::new(Mutex::new(ServiceHealth::Stopped));
        (Arc::new(FakeService { name, events, health: Arc::clone(&health) }), health)
    }

    fn poa_config() -> ConsensusConfig {
        let signer = CommitteeSigner::new("authority", [7; 32]).unwrap();
        ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
            authorities: vec![Authority { id: "authority".to_owned(), public_key: signer.ckb_public_key() }],
        })
    }

    #[test]
    fn registered_adapter_exposes_one_committed_descriptor() {
        let verifier = RegisteredFinalityVerifier::new(poa_config()).unwrap();
        let descriptor = verifier.descriptor();
        assert_eq!(descriptor.consensus_kind, myelin_consensus::ConsensusKind::ProofOfAuthority);
        assert_ne!(descriptor.config_commitment, [0; 32]);
        assert_ne!(descriptor.commitment(), [0; 32]);
        assert_eq!(ConsensusBinding::from_config(poa_config()).unwrap().consensus_module_commitment, descriptor.commitment());

        let message = EncodedConsensusMessage { format_version: 3, type_tag: 9, payload: vec![1] };
        assert_eq!(consensus_message_type(&message), MessageType::new(MessageClass::Consensus, 3, 9));
    }

    #[tokio::test]
    async fn supervisor_starts_topologically_and_stops_in_reverse() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (store, _) = fake("store", Arc::clone(&events));
        let (network, _) = fake("network", Arc::clone(&events));
        let (driver, _) = fake("driver", Arc::clone(&events));
        let mut supervisor = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        supervisor.register("driver", vec!["network".to_owned()], ServiceCriticality::Critical, driver).unwrap();
        supervisor.register("store", vec![], ServiceCriticality::Critical, store).unwrap();
        supervisor.register("network", vec!["store".to_owned()], ServiceCriticality::Restartable, network).unwrap();

        assert_eq!(supervisor.start_all().await.unwrap(), vec!["store", "network", "driver"]);
        assert_eq!(supervisor.stop_all().await.unwrap(), vec!["driver", "network", "store"]);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "start:store",
                "ready:store",
                "start:network",
                "ready:network",
                "start:driver",
                "ready:driver",
                "stop:driver",
                "stop:network",
                "stop:store",
            ]
        );
    }

    #[tokio::test]
    async fn critical_health_failure_stops_every_started_service() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (store, store_health) = fake("store", Arc::clone(&events));
        let (metrics, _) = fake("metrics", Arc::clone(&events));
        let mut supervisor = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        supervisor.register("store", vec![], ServiceCriticality::Critical, store).unwrap();
        supervisor.register("metrics", vec!["store".to_owned()], ServiceCriticality::Optional, metrics).unwrap();
        supervisor.start_all().await.unwrap();
        *store_health.lock().unwrap() = ServiceHealth::Failed("disk".to_owned());
        assert!(matches!(supervisor.enforce_health().await, Err(RuntimeError::CriticalService { .. })));
        let events = events.lock().unwrap();
        assert!(events.ends_with(&["stop:metrics".to_owned(), "stop:store".to_owned()]));
    }

    #[test]
    fn topology_rejects_unknown_dependencies_and_cycles() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (a, _) = fake("a", Arc::clone(&events));
        let (b, _) = fake("b", events);
        let mut unknown = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        unknown.register("a", vec!["missing".to_owned()], ServiceCriticality::Critical, a).unwrap();
        assert!(matches!(unknown.topological_order(), Err(RuntimeError::Topology(_))));

        let mut cycle = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        cycle.register("a", vec!["b".to_owned()], ServiceCriticality::Critical, b.clone()).unwrap();
        cycle.register("b", vec!["a".to_owned()], ServiceCriticality::Critical, b).unwrap();
        assert!(matches!(cycle.topological_order(), Err(RuntimeError::Topology(_))));
    }

    struct PanicOnStart;

    #[async_trait]
    impl ManagedService for PanicOnStart {
        async fn start(&self) -> Result<(), ManagedServiceError> {
            panic!("contained service panic")
        }

        async fn await_ready(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        fn health(&self) -> ServiceHealth {
            ServiceHealth::Failed("panic".to_owned())
        }
    }

    struct HungOnStart;

    #[async_trait]
    impl ManagedService for HungOnStart {
        async fn start(&self) -> Result<(), ManagedServiceError> {
            std::future::pending().await
        }

        async fn await_ready(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        fn health(&self) -> ServiceHealth {
            ServiceHealth::Starting
        }
    }

    #[tokio::test]
    async fn supervisor_contains_panics_and_bounds_hung_services() {
        let mut panicking = ServiceSupervisor::new(Duration::from_millis(20)).unwrap();
        panicking.register("panic", vec![], ServiceCriticality::Critical, Arc::new(PanicOnStart)).unwrap();
        assert!(matches!(panicking.start_all().await, Err(RuntimeError::Service { phase: "start", .. })));

        let mut hung = ServiceSupervisor::new(Duration::from_millis(20)).unwrap();
        hung.register("hung", vec![], ServiceCriticality::Critical, Arc::new(HungOnStart)).unwrap();
        assert!(matches!(hung.start_all().await, Err(RuntimeError::Service { phase: "start", .. })));
    }

    struct FailsOnStart;

    #[async_trait]
    impl ManagedService for FailsOnStart {
        async fn start(&self) -> Result<(), ManagedServiceError> {
            Err(ManagedServiceError::new("optional service unavailable"))
        }

        async fn await_ready(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), ManagedServiceError> {
            Ok(())
        }

        fn health(&self) -> ServiceHealth {
            ServiceHealth::Failed("unavailable".to_owned())
        }
    }

    #[tokio::test]
    async fn optional_start_failure_does_not_stop_an_independent_critical_service() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (writer, _) = fake("writer", events);
        let mut supervisor = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        supervisor.register("metrics", vec![], ServiceCriticality::Optional, Arc::new(FailsOnStart)).unwrap();
        supervisor.register("writer", vec![], ServiceCriticality::Critical, writer).unwrap();

        assert_eq!(supervisor.start_all().await.unwrap(), vec!["writer"]);
        let reports = supervisor.enforce_health().await.unwrap();
        assert!(reports.iter().any(|report| {
            report.name == "metrics"
                && report.criticality == ServiceCriticality::Optional
                && matches!(report.health, ServiceHealth::Failed(_))
        }));
        assert!(reports.iter().any(|report| report.name == "writer" && report.health == ServiceHealth::Ready));
    }

    #[derive(Clone)]
    struct RootExecutor(Hash32);

    impl TransitionExecutor for RootExecutor {
        fn state_root(&self) -> Hash32 {
            self.0
        }

        fn execute_block(&mut self, _height: u64, _transactions: &[Vec<u8>]) -> std::result::Result<ExecutionOutcome, String> {
            Ok(ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox: vec![] })
        }

        fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
            Ok(self.0.to_vec())
        }
    }

    #[derive(Default)]
    struct MinimalStore {
        genesis: Mutex<Option<SessionGenesis>>,
        head: Mutex<Option<SessionHead>>,
    }

    impl SessionStore for MinimalStore {
        fn create_session(&self, genesis: &SessionGenesis, head: &SessionHead) -> std::result::Result<(), StoreError> {
            *self.genesis.lock().unwrap() = Some(genesis.clone());
            *self.head.lock().unwrap() = Some(head.clone());
            Ok(())
        }

        fn load_genesis(&self, _session_id: Hash32) -> std::result::Result<SessionGenesis, StoreError> {
            self.genesis.lock().unwrap().clone().ok_or_else(|| StoreError::NotFound("genesis".to_owned()))
        }

        fn load_head(&self, _session_id: Hash32) -> std::result::Result<SessionHead, StoreError> {
            self.head.lock().unwrap().clone().ok_or_else(|| StoreError::NotFound("head".to_owned()))
        }

        fn load_chain(&self, _session_id: Hash32) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
            Ok(vec![])
        }

        fn commit_block(
            &self,
            _expected_head: &SessionHead,
            _new_head: &SessionHead,
            _record: &FinalisedBlockRecord,
        ) -> std::result::Result<(), StoreError> {
            Ok(())
        }

        fn load_consensus_wal(&self, _session_id: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError> {
            Ok(None)
        }

        fn compare_and_set_consensus_wal(
            &self,
            _expected_revision: Option<u64>,
            _wal: &ConsensusWal,
        ) -> std::result::Result<(), StoreError> {
            Ok(())
        }

        fn pending_outbox(&self, _session_id: Hash32, _limit: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError> {
            Ok(vec![])
        }

        fn acknowledge_outbox(&self, _session_id: Hash32, _message_id: Hash32) -> std::result::Result<(), StoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn runtime_host_gates_writes_on_critical_service_health() {
        let consensus_config = poa_config();
        let binding = ConsensusBinding::from_config(consensus_config.clone()).unwrap();
        let session_config = SessionConfig {
            session_id: [2; 32],
            consensus_kind: binding.consensus_kind,
            consensus_config_commitment: binding.consensus_config_commitment,
            consensus_module_commitment: binding.consensus_module_commitment,
            consensus_wal_schema_hash: binding.consensus_wal_schema_hash,
            initial_state_root: [1; 32],
            max_block_transactions: 1,
            max_block_bytes: 1,
            max_future_drift_ms: 1,
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let (writer, writer_health) = fake("writer", events);
        let mut supervisor = ServiceSupervisor::new(Duration::from_secs(1)).unwrap();
        supervisor.register("writer", vec![], ServiceCriticality::Critical, writer).unwrap();
        let mut host = RuntimeHost::create(
            session_config,
            consensus_config,
            RootExecutor([1; 32]),
            Arc::new(MinimalStore::default()),
            supervisor,
        )
        .unwrap();

        assert!(matches!(host.chain_mut(), Err(RuntimeError::WriterDisabled)));
        host.start_services().await.unwrap();
        assert!(host.writer_enabled());
        host.chain_mut().unwrap();

        *writer_health.lock().unwrap() = ServiceHealth::Failed("writer stopped".to_owned());
        assert!(matches!(host.chain_mut(), Err(RuntimeError::CriticalService { .. })));
        assert!(!host.writer_enabled());
        assert!(matches!(host.enforce_health().await, Err(RuntimeError::CriticalService { .. })));
        assert!(!host.writer_enabled());
        assert!(matches!(host.chain_mut(), Err(RuntimeError::WriterDisabled)));
    }
}
