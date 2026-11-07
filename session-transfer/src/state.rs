use crate::*;
use myelin_session::{ExecutionOutcome, PendingOutboxMessage, ProtocolEffectsContext, SessionHandoff, TransitionExecutor};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetAccount {
    pub asset: AssetId,
    pub owner: Hash32,
    pub available: u64,
    pub locked: u64,
    pub version: u64,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub components: Vec<u8>,
}

/// A source lock is keyed by the signature-independent transfer ID. It cannot
/// contain the eventual handoff/block hash: those include the resulting state
/// root and would make the state commitment circular.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingLock {
    pub transfer_id: Hash32,
    pub source: SourceIdentity,
    pub asset: AssetId,
    pub owner: Hash32,
    pub recipient: Hash32,
    pub amount: u64,
    pub nonce: Hash32,
    pub source_version: u64,
    pub components_hash: Hash32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub transfer_id: Hash32,
    pub handoff_id: Hash32,
    pub source: SourceIdentity,
    pub source_height: u64,
    pub source_state_root: Hash32,
    pub asset: AssetId,
    pub recipient: Hash32,
    pub amount: u64,
    pub kind: TransferKind,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub components: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferState {
    policy: TransferPolicy,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    accounts: Vec<AssetAccount>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    pending: Vec<PendingLock>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    receipts: Vec<Receipt>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    used_nonces: Vec<(Hash32, Hash32)>,
}
impl TransferState {
    pub fn new(policy: TransferPolicy, mut accounts: Vec<AssetAccount>) -> Result<Self> {
        accounts.sort_by_key(|a| (a.asset, a.owner));
        let state = Self { policy, accounts, pending: vec![], receipts: vec![], used_nonces: vec![] };
        state.validate()?;
        Ok(state)
    }
    pub fn policy(&self) -> &TransferPolicy {
        &self.policy
    }
    pub fn accounts(&self) -> &[AssetAccount] {
        &self.accounts
    }
    pub fn pending(&self) -> &[PendingLock] {
        &self.pending
    }
    pub fn receipts(&self) -> &[Receipt] {
        &self.receipts
    }
    pub(crate) fn owners(&self) -> Vec<Hash32> {
        self.accounts
            .iter()
            .map(|a| a.owner)
            .chain(self.receipts.iter().map(|r| r.recipient))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn account(&self, asset: AssetId, owner: Hash32) -> Option<&AssetAccount> {
        self.accounts.binary_search_by_key(&(asset, owner), |a| (a.asset, a.owner)).ok().map(|i| &self.accounts[i])
    }
    pub fn root(&self) -> Result<Hash32> {
        self.validate()?;
        codec::hash(b"myelin:reference-transfer-state-root:v1", self)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        codec::encode(b"MTRS", self, bounded_codec::MAX_RECORD_BYTES)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let state: Self = codec::decode(b"MTRS", bytes, bounded_codec::MAX_RECORD_BYTES)?;
        state.validate()?;
        Ok(state)
    }
    fn validate(&self) -> Result<()> {
        self.policy.validate()?;
        let limits = &self.policy.limits;
        if self.accounts.len() > limits.max_accounts as usize {
            return Err(Error::Bounds("accounts"));
        }
        if self.pending.len() > limits.max_pending as usize {
            return Err(Error::Bounds("pending locks"));
        }
        if self.receipts.len() > limits.max_receipts as usize {
            return Err(Error::Bounds("receipt nullifiers"));
        }
        if self.used_nonces.len() > limits.max_nonces as usize {
            return Err(Error::Bounds("source nonces"));
        }
        if self.accounts.windows(2).any(|w| (w[0].asset, w[0].owner) >= (w[1].asset, w[1].owner))
            || self.pending.windows(2).any(|w| w[0].transfer_id >= w[1].transfer_id)
            || self.receipts.windows(2).any(|w| w[0].transfer_id >= w[1].transfer_id)
            || self.used_nonces.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(Error::State("state indexes must be sorted and unique".into()));
        }
        let mut locked = std::collections::BTreeMap::<(AssetId, Hash32), u64>::new();
        for pending in &self.pending {
            pending.asset.validate()?;
            pending.source.validate()?;
            if self.policy.kind != TransferKind::Scarce
                || pending.amount == 0
                || [pending.transfer_id, pending.owner, pending.recipient, pending.nonce].contains(&[0; 32])
            {
                return Err(Error::State("invalid pending lock".into()));
            }
            let account =
                self.account(pending.asset, pending.owner).ok_or_else(|| Error::State("lock has no source account".into()))?;
            if pending.source_version >= account.version
                || *blake3::hash(&account.components).as_bytes() != pending.components_hash
                || self.used_nonces.binary_search(&(pending.owner, pending.nonce)).is_err()
            {
                return Err(Error::State("pending lock does not match the account/version/nonce".into()));
            }
            let value = locked.entry((pending.asset, pending.owner)).or_default();
            *value = value.checked_add(pending.amount).ok_or_else(|| Error::State("pending amount overflow".into()))?;
        }
        for account in &self.accounts {
            account.asset.validate()?;
            XOnlyPublicKey::from_slice(&account.owner)
                .map_err(|_| Error::State("account owner is not a Schnorr public key".into()))?;
            if account.owner == [0; 32]
                || account.components.len() > limits.max_components_bytes as usize
                || account.available.checked_add(account.locked).is_none()
                || locked.get(&(account.asset, account.owner)).copied().unwrap_or(0) != account.locked
            {
                return Err(Error::State("invalid account or locked amount does not equal pending locks".into()));
            }
        }
        let mut handoffs = std::collections::HashSet::new();
        for receipt in &self.receipts {
            receipt.asset.validate()?;
            receipt.source.validate()?;
            if receipt.kind != self.policy.kind
                || receipt.amount == 0
                || [receipt.transfer_id, receipt.handoff_id, receipt.recipient, receipt.source_state_root].contains(&[0; 32])
                || receipt.components.len() > limits.max_components_bytes as usize
                || !handoffs.insert(receipt.handoff_id)
                || (receipt.kind == TransferKind::CopyableReceipt && receipt.amount != 1)
            {
                return Err(Error::State("invalid receipt/nullifier".into()));
            }
        }
        if self.used_nonces.iter().any(|(owner, nonce)| *owner == [0; 32] || *nonce == [0; 32]) {
            return Err(Error::State("zero used nonce".into()));
        }
        Ok(())
    }

    fn issue(&mut self, binding: &SourceIdentity, intent: &HandoffIntent, expected_id: Hash32) -> Result<TransferEvent> {
        let (id, payload) = verify_intent(intent)?;
        if let TransferCoordination::Remote { terminal } = &self.policy.coordination {
            if binding.session_id != terminal.source_session
                || !matches!(payload.target, TransferTarget::Session { session_id, .. } if session_id == terminal.target_session)
            {
                return Err(Error::Unsupported("remote forwarding or another target"));
            }
        }
        self.policy.matches_payload(&payload)?;
        if payload.source != *binding || id != expected_id {
            return Err(Error::Authorization("source genesis or command transfer ID mismatch".into()));
        }
        let nonce_index = self
            .used_nonces
            .binary_search(&(payload.owner, payload.nonce))
            .err()
            .ok_or_else(|| Error::Authorization("owner nonce already used".into()))?;
        let index = self
            .accounts
            .binary_search_by_key(&(payload.asset, payload.owner), |a| (a.asset, a.owner))
            .map_err(|_| Error::Authorization("actor does not own the source object".into()))?;
        let account = &mut self.accounts[index];
        if account.version != payload.source_version || account.components != payload.components || account.available < payload.amount
        {
            return Err(Error::Authorization("source object version, components or available amount mismatch".into()));
        }
        if payload.kind == TransferKind::Scarce {
            account.available =
                account.available.checked_sub(payload.amount).ok_or_else(|| Error::State("insufficient balance".into()))?;
            account.locked = account.locked.checked_add(payload.amount).ok_or_else(|| Error::State("lock overflow".into()))?;
            let pending_index = self
                .pending
                .binary_search_by_key(&id, |p| p.transfer_id)
                .err()
                .ok_or_else(|| Error::Authorization("transfer already locked".into()))?;
            self.pending.insert(
                pending_index,
                PendingLock {
                    transfer_id: id,
                    source: binding.clone(),
                    asset: payload.asset,
                    owner: payload.owner,
                    recipient: payload.recipient,
                    amount: payload.amount,
                    nonce: payload.nonce,
                    source_version: payload.source_version,
                    components_hash: *blake3::hash(&payload.components).as_bytes(),
                },
            );
        }
        account.version = account.version.checked_add(1).ok_or_else(|| Error::State("object version overflow".into()))?;
        self.used_nonces.insert(nonce_index, (payload.owner, payload.nonce));
        Ok(TransferEvent::Issued { transfer_id: id, kind: payload.kind, amount: payload.amount })
    }

    fn lifecycle(
        &mut self,
        binding: &SourceIdentity,
        action: &myelin_session::ResolvedHandoffAction,
        authorization: Option<&[u8]>,
    ) -> Result<TransferEvent> {
        let (transfer_id, payload) = verify_handoff(&action.handoff)?;
        self.policy.matches_payload(&payload)?;
        if payload.kind != TransferKind::Scarce
            || payload.source.session_id != action.handoff.source_session_id
            || payload.source.application_profile_commitment != action.handoff.source_application_profile_commitment
            || payload.source.consensus_module_commitment != action.handoff.source_consensus_module_commitment
        {
            return Err(Error::Authorization("lifecycle requires the original source's scarce lock".into()));
        }
        let index = self
            .pending
            .binary_search_by_key(&transfer_id, |p| p.transfer_id)
            .map_err(|_| Error::Authorization("no pending source lock for lifecycle action".into()))?;
        let lock = &self.pending[index];
        if lock.source != payload.source
            || lock.asset != payload.asset
            || lock.owner != payload.owner
            || lock.recipient != payload.recipient
            || lock.amount != payload.amount
            || lock.nonce != payload.nonce
            || lock.source_version != payload.source_version
            || lock.components_hash != *blake3::hash(&payload.components).as_bytes()
        {
            return Err(Error::Authorization("lifecycle witness does not match exact source lock".into()));
        }
        let handoff_id = action.handoff.id();
        match (&action.action, authorization) {
            (myelin_session::HandoffAction::Cancel { .. }, Some(signature)) if action.cancelled_at.is_none() => {
                let digest = cancellation_digest(binding.genesis_commitment, transfer_id, handoff_id, payload.owner)?;
                let key = XOnlyPublicKey::from_slice(&payload.owner).map_err(|e| Error::Authorization(e.to_string()))?;
                let sig = Signature::from_slice(signature).map_err(|e| Error::Authorization(e.to_string()))?;
                Secp256k1::verification_only()
                    .verify_schnorr(&sig, &Message::from_digest(digest), &key)
                    .map_err(|_| Error::Authorization("cancellation owner signature is invalid".into()))?;
                Ok(TransferEvent::Cancelled { transfer_id, handoff_id })
            }
            (myelin_session::HandoffAction::Reclaim { .. }, None) if action.cancelled_at.is_some() => {
                let i = self
                    .accounts
                    .binary_search_by_key(&(payload.asset, payload.owner), |a| (a.asset, a.owner))
                    .map_err(|_| Error::State("refund owner account absent".into()))?;
                let account = &mut self.accounts[i];
                account.locked =
                    account.locked.checked_sub(payload.amount).ok_or_else(|| Error::State("refund exceeds lock".into()))?;
                account.available =
                    account.available.checked_add(payload.amount).ok_or_else(|| Error::State("refund overflow".into()))?;
                account.version = account.version.checked_add(1).ok_or_else(|| Error::State("refund version overflow".into()))?;
                self.pending.remove(index);
                Ok(TransferEvent::Reclaimed { transfer_id, handoff_id, amount: payload.amount })
            }
            (myelin_session::HandoffAction::Retire { .. } | myelin_session::HandoffAction::RemoteRetire { .. }, None)
                if action.cancelled_at.is_none() =>
            {
                let i = self
                    .accounts
                    .binary_search_by_key(&(payload.asset, payload.owner), |a| (a.asset, a.owner))
                    .map_err(|_| Error::State("retirement owner account absent".into()))?;
                let account = &mut self.accounts[i];
                account.locked =
                    account.locked.checked_sub(payload.amount).ok_or_else(|| Error::State("retirement exceeds lock".into()))?;
                account.version = account.version.checked_add(1).ok_or_else(|| Error::State("retirement version overflow".into()))?;
                self.pending.remove(index);
                Ok(TransferEvent::Retired { transfer_id, handoff_id, amount: payload.amount })
            }
            _ => Err(Error::Authorization("lifecycle command/witness mismatch".into())),
        }
    }

    fn intake(
        &mut self,
        binding: &SourceIdentity,
        handoff: &SessionHandoff,
        expected_id: Hash32,
        timestamp_ms: u64,
    ) -> Result<TransferEvent> {
        let id = handoff.id();
        if id != expected_id || timestamp_ms > handoff.expires_at_ms {
            return Err(Error::Authorization("handoff identity or expiry mismatch".into()));
        }
        let intent = HandoffIntent {
            target: handoff.target.clone(),
            expires_at_ms: handoff.expires_at_ms,
            payload: handoff.payload.clone(),
            authorization: handoff.authorization.clone(),
            evidence_requirement: handoff.evidence_requirement.clone(),
        };
        let (transfer_id, payload) = verify_intent(&intent)?;
        self.policy.matches_payload(&payload)?;
        let anchor = self
            .policy
            .accepted_sources
            .binary_search_by_key(&payload.source.session_id, |s| s.session_id)
            .ok()
            .map(|i| &self.policy.accepted_sources[i]);
        if anchor != Some(&payload.source)
            || payload.source.session_id != handoff.source_session_id
            || payload.source.application_profile_commitment != handoff.source_application_profile_commitment
            || payload.source.consensus_module_commitment != handoff.source_consensus_module_commitment
        {
            return Err(Error::Source("handoff does not match a pinned source genesis/profile/authority".into()));
        }
        match &payload.target {
            TransferTarget::Session { session_id, genesis_commitment }
                if *session_id == binding.session_id && *genesis_commitment == binding.genesis_commitment => {}
            TransferTarget::SharedStorePolicy { policy_commitment } if *policy_commitment == self.policy.intake_policy()? => {}
            _ => return Err(Error::Authorization("target genesis or policy mismatch".into())),
        }
        let receipt_index = self
            .receipts
            .binary_search_by_key(&transfer_id, |r| r.transfer_id)
            .err()
            .ok_or_else(|| Error::Authorization("transfer nullifier already credited".into()))?;
        if payload.kind == TransferKind::Scarce {
            match self.accounts.binary_search_by_key(&(payload.asset, payload.recipient), |a| (a.asset, a.owner)) {
                Ok(index) => {
                    let account = &mut self.accounts[index];
                    if account.components != payload.components {
                        return Err(Error::State("target object components conflict".into()));
                    }
                    account.available =
                        account.available.checked_add(payload.amount).ok_or_else(|| Error::State("target amount overflow".into()))?;
                    account.version = account.version.checked_add(1).ok_or_else(|| Error::State("target version overflow".into()))?;
                }
                Err(index) => self.accounts.insert(
                    index,
                    AssetAccount {
                        asset: payload.asset,
                        owner: payload.recipient,
                        available: payload.amount,
                        locked: 0,
                        version: 0,
                        components: payload.components.clone(),
                    },
                ),
            }
        }
        self.receipts.insert(
            receipt_index,
            Receipt {
                transfer_id,
                handoff_id: id,
                source: payload.source,
                source_height: handoff.source_height,
                source_state_root: handoff.source_state_root,
                asset: payload.asset,
                recipient: payload.recipient,
                amount: payload.amount,
                kind: payload.kind,
                components: payload.components,
            },
        );
        Ok(TransferEvent::Received { transfer_id, handoff_id: id, kind: payload.kind, amount: payload.amount })
    }
}

#[derive(Serialize)]
enum TransferEvent {
    Issued { transfer_id: Hash32, kind: TransferKind, amount: u64 },
    Received { transfer_id: Hash32, handoff_id: Hash32, kind: TransferKind, amount: u64 },
    Cancelled { transfer_id: Hash32, handoff_id: Hash32 },
    Reclaimed { transfer_id: Hash32, handoff_id: Hash32, amount: u64 },
    Retired { transfer_id: Hash32, handoff_id: Hash32, amount: u64 },
    Continued { target_genesis: Hash32 },
}

/// Pure deterministic application executor. All storage mutation remains owned
/// by SessionChain. Source bindings must be re-established from the selected
/// store on construction/recovery; an arbitrary nonzero source hash is not enough.
#[derive(Clone)]
pub struct TransferExecutor {
    state: TransferState,
    binding: SourceIdentity,
    root: Hash32,
    sources: SourceBindings,
}
impl TransferExecutor {
    pub fn new(config: &SessionConfig, state: TransferState, sources: SourceBindings) -> Result<Self> {
        let executor = Self::bind(config, state, sources)?;
        if executor.root != config.initial_state_root {
            return Err(Error::State("initial state root differs from genesis".into()));
        }
        Ok(executor)
    }
    pub fn restore(config: &SessionConfig, bytes: &[u8], sources: SourceBindings) -> Result<Self> {
        Self::bind(config, TransferState::decode(bytes)?, sources)
    }
    fn bind(config: &SessionConfig, state: TransferState, sources: SourceBindings) -> Result<Self> {
        config.validate().map_err(|e| Error::Policy(e.to_string()))?;
        if sources.policy_commitment != state.policy.commitment()?
            || sources.inheritance.as_ref().is_some_and(|(owner, _)| *owner != config.commitment())
            || config.protocol_effects != state.policy.effect_policy()?
            || config.application_profile.input_schema_hash != input_schema()
            || !config.application_profile.command_identity_required
            || config.application_profile.state_codec_hash != state_codec()
            || config.application_profile.handoff_policy_hash != state.policy.intake_policy()?
        {
            return Err(Error::Policy("executor policy, source bindings or profile differs from genesis".into()));
        }
        if let TransferCoordination::Remote { terminal } = &state.policy.coordination {
            if config.predecessor.is_some()
                || (config.session_id != terminal.source_session && config.session_id != terminal.target_session)
            {
                return Err(Error::Unsupported("remote coordination has one original source and fixed target"));
            }
            sources
                .remote_registry
                .as_ref()
                .ok_or_else(|| Error::Policy("remote coordination requires independently configured terminal trust".into()))?
                .terminal_registration(config, terminal.commitment().map_err(|e| Error::Policy(e.to_string()))?)
                .map_err(|e| Error::Policy(e.to_string()))?;
        }
        Ok(Self { root: state.root()?, state, binding: SourceIdentity::from_config(config)?, sources })
    }
    pub fn state(&self) -> &TransferState {
        &self.state
    }
    pub fn execute_transfer(
        &mut self,
        context: &ProtocolEffectsContext,
        input: &FrameInput,
        consumed: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome> {
        self.execute_transfer_actions(context, &[], input, consumed, txs)
    }
    pub fn execute_transfer_actions(
        &mut self,
        context: &ProtocolEffectsContext,
        actions: &[myelin_session::ResolvedHandoffAction],
        input: &FrameInput,
        consumed: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome> {
        context.validate().map_err(|e| Error::Policy(e.to_string()))?;
        for action in actions {
            action.validate_for(context.source_session_id, context.height).map_err(|e| Error::Policy(e.to_string()))?;
        }
        if context.source_genesis_commitment != self.binding.genesis_commitment
            || context.source_session_id != self.binding.session_id
            || context.policy != self.state.policy.effect_policy()?
            || context.pre_state_root != self.root
            || context.input_root != input.root()
        {
            return Err(Error::Policy("execution context differs from bound genesis/state/input".into()));
        }
        if !txs.is_empty() {
            return Err(Error::Unsupported("reference transfer policy takes canonical host commands, not CellTx"));
        }
        let commands: Commands = codec::decode(b"MTRD", &input.payload, MAX_COMMAND_BYTES)?;
        let successor = context.successor().map_err(|e| Error::Policy(e.to_string()))?;
        if successor.is_some() && !matches!(commands.commands.as_slice(), [TransferCommand::Continue { .. }]) {
            return Err(Error::Authorization("continuation requires its own owner-authorized command".into()));
        }
        let count = commands.commands.len() as u64;
        if input.end_position.checked_sub(input.start_position) != Some(count)
            || input.logical_time_end.checked_sub(input.logical_time_start) != Some(count)
            || commands.commands.iter().map(|c| c.identity(self.binding.genesis_commitment)).collect::<Result<Vec<_>>>()?
                != input.commands
        {
            return Err(Error::Authorization("command identities/ranges do not match canonical commands".into()));
        }
        let mut seen = std::collections::HashSet::new();
        if input.commands.iter().any(|c| !seen.insert(c.id)) {
            return Err(Error::Authorization("duplicate command identity".into()));
        }
        let mut next = self.state.clone();
        let mut issued = context.issued_handoffs.iter();
        let mut incoming = consumed.iter();
        let mut lifecycle = actions.iter();
        let mut outcome =
            ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox: vec![], cycles: 0 };
        for command in commands.commands {
            let event = match command {
                TransferCommand::Issue { transfer_id } => next.issue(
                    &self.binding,
                    issued.next().ok_or_else(|| Error::Policy("missing issued handoff".into()))?,
                    transfer_id,
                )?,
                TransferCommand::Cancel { handoff_id, authorization } => {
                    if matches!(next.policy.coordination, TransferCoordination::Remote { .. }) {
                        return Err(Error::Authorization("remote cancellation requires its exclusive terminal certificate".into()));
                    }
                    let action = lifecycle.next().ok_or_else(|| Error::Policy("missing cancellation witness".into()))?;
                    if action.action != (myelin_session::HandoffAction::Cancel { handoff_id }) {
                        return Err(Error::Policy("wrong cancellation action".into()));
                    }
                    next.lifecycle(&self.binding, action, Some(&authorization))?
                }
                TransferCommand::RemoteCancel { handoff_id, authorization, certificate } => {
                    let TransferCoordination::Remote { terminal } = &next.policy.coordination else {
                        return Err(Error::Unsupported("remote cancellation requires remote coordination"));
                    };
                    if self.binding.session_id != terminal.source_session {
                        return Err(Error::Authorization("only the original remote source can cancel".into()));
                    }
                    let action = lifecycle.next().ok_or_else(|| Error::Policy("missing cancellation witness".into()))?;
                    if action.action != (myelin_session::HandoffAction::Cancel { handoff_id }) {
                        return Err(Error::Policy("wrong cancellation action".into()));
                    }
                    let (_, payload) = verify_handoff(&action.handoff)?;
                    let TransferTarget::Session { genesis_commitment: target, .. } = payload.target else {
                        return Err(Error::Unsupported("remote policy targets"));
                    };
                    let cert = myelin_session::RemoteTerminalCertificate::decode(&certificate)
                        .map_err(|e| Error::Authorization(e.to_string()))?;
                    let verified = cert
                        .verify(terminal, self.binding.genesis_commitment, target, handoff_id)
                        .map_err(|e| Error::Authorization(e.to_string()))?;
                    let digest =
                        myelin_session::remote_cancellation_digest(self.binding.genesis_commitment, handoff_id, &authorization)
                            .map_err(|e| Error::Authorization(e.to_string()))?;
                    if verified.decision() != &(myelin_session::RemoteTerminalDecision::Cancelled { authorization_digest: digest }) {
                        return Err(Error::Authorization("terminal decision does not cancel this exact owner authorization".into()));
                    }
                    next.lifecycle(&self.binding, action, Some(&authorization))?
                }
                TransferCommand::Reclaim { handoff_id } => {
                    let action = lifecycle.next().ok_or_else(|| Error::Policy("missing refund witness".into()))?;
                    if action.action != (myelin_session::HandoffAction::Reclaim { handoff_id }) {
                        return Err(Error::Policy("wrong refund action".into()));
                    }
                    next.lifecycle(&self.binding, action, None)?
                }
                TransferCommand::Retire { handoff_id } => {
                    let action = lifecycle.next().ok_or_else(|| Error::Policy("missing retirement witness".into()))?;
                    if !matches!(&action.action, myelin_session::HandoffAction::Retire { handoff_id: id, .. } | myelin_session::HandoffAction::RemoteRetire { handoff_id: id, .. } if *id == handoff_id)
                    {
                        return Err(Error::Policy("wrong retirement action".into()));
                    }
                    match (&next.policy.coordination, &action.action) {
                        (TransferCoordination::Remote { terminal }, myelin_session::HandoffAction::RemoteRetire { proof, .. }) => {
                            let registry = self
                                .sources
                                .remote_registry
                                .as_ref()
                                .ok_or_else(|| Error::Source("remote completion trust unavailable".into()))?;
                            let (source, _) = registry.configs(proof.import_policy).map_err(|e| Error::Source(e.to_string()))?;
                            if source.commitment() != self.binding.genesis_commitment || source.session_id != terminal.source_session {
                                return Err(Error::Authorization("remote retirement source mismatch".into()));
                            }
                            proof.verify(registry, source, &action.handoff).map_err(|e| Error::Authorization(e.to_string()))?;
                        }
                        (TransferCoordination::SharedStore, myelin_session::HandoffAction::Retire { .. }) => {}
                        _ => return Err(Error::Unsupported("retirement coordination/proof mismatch")),
                    }
                    next.lifecycle(&self.binding, action, None)?
                }
                TransferCommand::Intake { handoff_id } => next.intake(
                    &self.binding,
                    incoming.next().ok_or_else(|| Error::Policy("missing consumed handoff".into()))?,
                    handoff_id,
                    context.timestamp_ms,
                )?,
                TransferCommand::Continue { target_genesis, approvals } => {
                    if matches!(next.policy.coordination, TransferCoordination::Remote { .. }) {
                        return Err(Error::Unsupported("remote continuation and forwarding are disabled"));
                    }
                    let target = successor.as_ref().ok_or_else(|| Error::Policy("continuation has no successor config".into()))?;
                    if target_genesis != target.commitment()
                        || target.initial_state_root != self.root
                        || target.application_profile.commitment() != self.binding.application_profile_commitment
                        || target.protocol_effects != self.state.policy.effect_policy()?
                    {
                        return Err(Error::Unsupported(
                            "reference transfer continuation preserves the complete application and state",
                        ));
                    }
                    for lock in &self.state.pending {
                        let depth = if lock.source.session_id == self.binding.session_id {
                            0
                        } else {
                            *self
                                .sources
                                .inheritance
                                .as_ref()
                                .and_then(|(_, depths)| depths.get(&lock.source.session_id))
                                .ok_or(Error::Unsupported("continuation needs canonical inherited-lock lineage"))?
                        };
                        if depth >= myelin_session::MAX_HANDOFF_OWNER_DEPTH {
                            return Err(Error::Bounds("reconcile inherited handoffs before another continuation"));
                        }
                    }
                    let owners = self.state.owners();
                    if owners.is_empty() || approvals.iter().map(|a| a.owner).collect::<Vec<_>>() != owners {
                        return Err(Error::Authorization("continuation requires every owner exactly once in sorted order".into()));
                    }
                    let digest = continuation_digest(self.binding.genesis_commitment, self.root, target_genesis)?;
                    for approval in approvals {
                        let key = XOnlyPublicKey::from_slice(&approval.owner).map_err(|e| Error::Authorization(e.to_string()))?;
                        let signature = Signature::from_slice(&approval.signature).map_err(|e| Error::Authorization(e.to_string()))?;
                        Secp256k1::verification_only()
                            .verify_schnorr(&signature, &Message::from_digest(digest), &key)
                            .map_err(|_| Error::Authorization("continuation owner signature is invalid".into()))?;
                    }
                    TransferEvent::Continued { target_genesis }
                }
            };
            // Check state bounds after each action, before admitting another.
            next.validate()?;
            outcome.data_commitments.push(codec::hash(b"myelin:reference-transfer-event:v2", &event)?);
            outcome.outbox.push(PendingOutboxMessage {
                topic: "myelin/reference-transfer/v2".into(),
                payload: codec::encode(b"MTRF", &event, 1024)?,
            });
        }
        if issued.next().is_some() || incoming.next().is_some() || lifecycle.next().is_some() {
            return Err(Error::Policy("unmatched protocol effect in request".into()));
        }
        let root = next.root()?;
        self.state = next;
        self.root = root;
        Ok(outcome)
    }
}
impl TransitionExecutor for TransferExecutor {
    fn state_root(&self) -> Hash32 {
        self.root
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        self.state.encode().map_err(|e| e.to_string())
    }
    fn execute_block(
        &mut self,
        _: u64,
        _: &FrameInput,
        _: &[SessionHandoff],
        _: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        Err("reference transfer execution requires its authorized context".into())
    }
    fn execute_block_with_lifecycle(
        &mut self,
        c: &ProtocolEffectsContext,
        actions: &[myelin_session::ResolvedHandoffAction],
        f: &FrameInput,
        h: &[SessionHandoff],
        t: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        self.execute_transfer_actions(c, actions, f, h, t).map_err(|e| e.to_string())
    }
    fn execute_block_with_context(
        &mut self,
        c: &ProtocolEffectsContext,
        f: &FrameInput,
        h: &[SessionHandoff],
        t: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        self.execute_transfer(c, f, h, t).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_envelopes_match_independent_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/protocol/retirement.json")).unwrap();
        let id = serde_json::from_value(v["resolved_action"]["action"]["Retire"]["handoff_id"].clone()).unwrap();
        let event = TransferEvent::Retired { transfer_id: [8; 32], handoff_id: id, amount: 25 };
        let hex = |b: &[u8]| b.iter().map(|v| format!("{v:02x}")).collect::<String>();
        assert_eq!(hex(&codec::hash(b"myelin:reference-transfer-event:v2", &event).unwrap()), v["event"]["commitment"]);
        assert_eq!(hex(&codec::encode(b"MTRF", &event, 1024).unwrap()), v["event_envelope_hex"]);
        let commands = Commands { commands: vec![TransferCommand::Retire { handoff_id: id }] };
        assert_eq!(hex(&codec::encode(b"MTRD", &commands, 1024).unwrap()), v["command_envelope_hex"]);
    }
}
