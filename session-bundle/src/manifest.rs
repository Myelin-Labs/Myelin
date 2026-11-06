use crate::{BundleError, ErrorCode, Result};
use myelin_session::{ApplicationProfile, Hash32};
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const FORMAT: &str = "myelin-session-bundle-v2";
const EXECUTION_DOMAIN: &[u8] = b"myelin:bundle-execution:v2";
const MANIFEST_DOMAIN: &[u8] = b"myelin:bundle-manifest:v2";

/// Lowercase BLAKE3 hex is also the complete blob filename. No supplied path is
/// interpreted by the importer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId(pub Hash32);
impl BlobId {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }
    pub fn hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write;
            write!(&mut out, "{byte:02x}").expect("writing to a string");
        }
        out
    }
}
impl Serialize for BlobId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.hex())
    }
}
impl<'de> Deserialize<'de> for BlobId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 64 || !text.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return Err(D::Error::custom("blob id must be exactly 64 lowercase hex characters"));
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(D::Error::custom)?;
        }
        Ok(Self(bytes))
    }
}

/// Operational import budgets. These cannot weaken genesis protocol budgets.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleLimits {
    pub manifest_bytes: u64,
    pub blob_bytes: u64,
    pub total_bytes: u64,
    pub nodes: usize,
    pub edges: usize,
    /// Longest blob dependency path, including consecutive history blocks.
    pub dependency_depth: usize,
    pub sessions: usize,
    pub blocks: usize,
}
impl Default for BundleLimits {
    fn default() -> Self {
        Self {
            manifest_bytes: 8 * 1024 * 1024,
            blob_bytes: 256 * 1024 * 1024,
            total_bytes: 8 * 1024 * 1024 * 1024,
            nodes: 65_536,
            edges: 262_144,
            dependency_depth: 1024,
            sessions: 256,
            blocks: 60_000,
        }
    }
}
impl BundleLimits {
    pub fn validate(&self) -> Result<()> {
        if self.manifest_bytes == 0
            || self.manifest_bytes > 32 * 1024 * 1024
            || self.blob_bytes == 0
            || self.blob_bytes > 512 * 1024 * 1024
            || self.total_bytes == 0
            || self.total_bytes > 64 * 1024 * 1024 * 1024
            || self.nodes == 0
            || self.nodes > 262_144
            || self.edges == 0
            || self.edges > 1_048_576
            || self.dependency_depth == 0
            // Traversal is iterative; depth shares the absolute node ceiling.
            // The caller's smaller depth allowance is still enforced per DAG.
            || self.dependency_depth > 262_144
            || self.sessions == 0
            || self.sessions > 4096
            || self.blocks == 0
            || self.blocks > self.nodes
        {
            return Err(BundleError::new(ErrorCode::Limit, "invalid or excessive bundle import limits"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlobKind {
    Genesis,
    Head,
    Checkpoint,
    Block,
    Handoff,
    Evidence,
    Consensus,
    Program,
    Replayer,
    Rules,
    BuildInputs,
    Toolchain,
    RuntimeDependency,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub id: BlobId,
    pub bytes: u64,
    /// One content blob can have several roles (e.g. program and replayer).
    pub roles: Vec<BlobKind>,
    /// Content prerequisites, not reverse indexes or storage ownership links.
    pub dependencies: Vec<BlobId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionArtifacts {
    pub program: BlobId,
    pub replayer: BlobId,
    pub rules: BlobId,
    /// Current canonical FilePackage root; opaque metadata is not accepted.
    pub build_inputs: BlobId,
    /// Current canonical FilePackage root; may share the complete build kit.
    pub toolchain: BlobId,
    pub runtime_dependencies: Vec<BlobId>,
    /// Descriptive ABI/target, verified against the actual runner before use.
    pub target: String,
    pub runner_abi: String,
    /// Required runtime layout when using the isolated runner.
    pub runtime: Option<crate::runner::RuntimeLayout>,
}
impl ExecutionArtifacts {
    pub fn commitment(&self) -> Hash32 {
        let mut hash = blake3::Hasher::new();
        hash.update(EXECUTION_DOMAIN);
        hash.update(&serde_json::to_vec(self).expect("execution artifacts serialize"));
        *hash.finalize().as_bytes()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInventory {
    /// External deployment/network scope, recognized only by the trust file.
    pub network: Hash32,
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub profile: ApplicationProfile,
    pub genesis: BlobId,
    pub consensus: BlobId,
    pub head: BlobId,
    pub checkpoint: BlobId,
    /// Full genesis-prefix inventory, ascending block height. Range replay may
    /// later select a suffix but must report its checkpoint assurance separately.
    pub blocks: Vec<BlobId>,
    pub handoffs: Vec<BlobId>,
    pub evidence: Vec<BlobId>,
    pub execution: ExecutionArtifacts,
}

/// Obtained independently of a bundle. A key in a downloaded bundle is never
/// sufficient to recognize an official application, deployment or old replayer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustAnchor {
    pub network: Hash32,
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub profile_commitment: Hash32,
    pub execution_commitment: Hash32,
}
impl SessionInventory {
    /// Produce the identity to review out of band. Calling this is NOT approval.
    pub fn proposed_anchor(&self) -> TrustAnchor {
        TrustAnchor {
            network: self.network,
            session_id: self.session_id,
            genesis_commitment: self.genesis_commitment,
            profile_commitment: self.profile.commitment(),
            execution_commitment: self.execution.commitment(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub format: String,
    pub myelin_version: String,
    pub record_versions: Vec<u16>,
    pub roots: Vec<Hash32>,
    /// Strictly ascending by digest; no duplicate blobs or unused extra blobs.
    pub blobs: Vec<Blob>,
    /// Strictly ascending by session id; each requires independent recognition.
    pub sessions: Vec<SessionInventory>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InventorySummary {
    pub manifest_commitment: Hash32,
    pub sessions: usize,
    pub blobs: usize,
    pub bytes: u64,
    pub edges: usize,
    pub dependency_depth: usize,
}

fn invalid(message: impl Into<String>) -> BundleError {
    BundleError::new(ErrorCode::Format, message)
}
fn sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

impl BundleManifest {
    pub fn decode(bytes: &[u8], limits: &BundleLimits) -> Result<Self> {
        limits.validate()?;
        if bytes.len() as u64 > limits.manifest_bytes {
            return Err(BundleError::new(ErrorCode::Limit, "manifest byte budget exceeded"));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        // Enforce one canonical encoding; duplicate keys, whitespace drift and
        // alternate number/string spellings cannot create multiple identities.
        if manifest.encode()? != bytes {
            return Err(invalid("noncanonical bundle manifest"));
        }
        manifest.validate_structure(limits)?;
        Ok(manifest)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| invalid(e.to_string()))
    }
    pub fn commitment(&self) -> Result<Hash32> {
        let mut hash = blake3::Hasher::new();
        hash.update(MANIFEST_DOMAIN);
        hash.update(&self.encode()?);
        Ok(*hash.finalize().as_bytes())
    }
    pub fn recognize(&self, anchors: &[TrustAnchor]) -> Result<()> {
        for session in &self.sessions {
            crate::work::io(0)?;
            if !anchors.contains(&session.proposed_anchor()) {
                return Err(BundleError::new(
                    ErrorCode::UnknownTrustAnchor,
                    format!("no external genesis/profile/execution anchor for session {}", BlobId(session.session_id).hex()),
                ));
            }
        }
        Ok(())
    }
    pub fn validate_structure(&self, limits: &BundleLimits) -> Result<InventorySummary> {
        limits.validate()?;
        if self.format != FORMAT
            || self.myelin_version.is_empty()
            || self.myelin_version.len() > 64
            || self.record_versions != [myelin_session::RECORD_FORMAT_VERSION]
        {
            return Err(invalid("unsupported format, record version or missing Myelin version"));
        }
        if self.encode()?.len() as u64 > limits.manifest_bytes
            || self.blobs.len() > limits.nodes
            || self.sessions.len() > limits.sessions
        {
            return Err(BundleError::new(ErrorCode::Limit, "manifest node/session/byte budget exceeded"));
        }
        if self.blobs.is_empty()
            || self.sessions.is_empty()
            || self.roots.is_empty()
            || !sorted_unique(&self.roots)
            || !self.blobs.windows(2).all(|p| p[0].id < p[1].id)
            || !self.sessions.windows(2).all(|p| p[0].session_id < p[1].session_id)
        {
            return Err(invalid("empty, duplicated or unsorted inventory"));
        }
        let index: BTreeMap<_, _> = self.blobs.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        let mut used = BTreeSet::new();
        let mut require = |id: BlobId, role: BlobKind| -> Result<()> {
            let i = index
                .get(&id)
                .ok_or_else(|| BundleError::new(ErrorCode::MissingDependency, format!("missing {role:?} descriptor {}", id.hex())))?;
            if !self.blobs[*i].roles.contains(&role) {
                return Err(invalid(format!("blob {} does not declare role {role:?}", id.hex())));
            }
            used.insert(id);
            Ok(())
        };
        let mut block_count = 0usize;
        for s in &self.sessions {
            crate::work::io(0)?;
            if [s.network, s.session_id, s.genesis_commitment].contains(&[0; 32]) {
                return Err(invalid("empty deployment or session identity"));
            }
            s.profile.validate().map_err(|e| invalid(e.to_string()))?;
            block_count = block_count.checked_add(s.blocks.len()).ok_or_else(|| invalid("block count overflow"))?;
            if block_count > limits.blocks {
                return Err(BundleError::new(ErrorCode::Limit, "block inventory budget exceeded"));
            }
            for (id, role) in [
                (s.genesis, BlobKind::Genesis),
                (s.consensus, BlobKind::Consensus),
                (s.head, BlobKind::Head),
                (s.checkpoint, BlobKind::Checkpoint),
                (s.execution.program, BlobKind::Program),
                (s.execution.replayer, BlobKind::Replayer),
                (s.execution.rules, BlobKind::Rules),
                (s.execution.build_inputs, BlobKind::BuildInputs),
                (s.execution.toolchain, BlobKind::Toolchain),
            ] {
                require(id, role)?;
            }
            for (ids, role) in [
                (&s.blocks, BlobKind::Block),
                (&s.handoffs, BlobKind::Handoff),
                (&s.evidence, BlobKind::Evidence),
                (&s.execution.runtime_dependencies, BlobKind::RuntimeDependency),
            ] {
                let mut seen = BTreeSet::new();
                for id in ids {
                    crate::work::io(0)?;
                    if !seen.insert(*id) {
                        return Err(invalid("duplicate artifact in one session role"));
                    }
                    require(*id, role)?;
                }
            }
            if s.execution.target.is_empty()
                || s.execution.target.len() > 128
                || s.execution.runner_abi.is_empty()
                || s.execution.runner_abi.len() > 128
                || !sorted_unique(&s.execution.runtime_dependencies)
            {
                return Err(invalid("invalid runner target, ABI or dependency ordering"));
            }
            if s.profile.program_digest != s.execution.program.0 {
                return Err(BundleError::new(ErrorCode::Integrity, "program artifact differs from the genesis profile"));
            }
            if let Some(runtime) = &s.execution.runtime {
                runtime.validate(&s.execution.runtime_dependencies)?;
            }
        }
        for root in &self.roots {
            crate::work::io(0)?;
            if !self.sessions.iter().any(|s| s.session_id == *root) {
                return Err(BundleError::new(ErrorCode::MissingDependency, "root session is absent"));
            }
        }
        let mut total = 0u64;
        let mut edges = 0usize;
        let mut remaining = vec![0usize; self.blobs.len()];
        let mut dependents = vec![vec![]; self.blobs.len()];
        for (i, blob) in self.blobs.iter().enumerate() {
            crate::work::io(0)?;
            if blob.bytes == 0 || blob.bytes > limits.blob_bytes || blob.roles.is_empty() || blob.roles.len() > 14 {
                return Err(BundleError::new(ErrorCode::Limit, "blob size or role count outside limits"));
            }
            for (j, role) in blob.roles.iter().enumerate() {
                if blob.roles[..j].contains(role) {
                    return Err(invalid("duplicate blob role"));
                }
            }
            total = total.checked_add(blob.bytes).ok_or_else(|| invalid("expanded size overflow"))?;
            edges = edges.checked_add(blob.dependencies.len()).ok_or_else(|| invalid("edge count overflow"))?;
            if total > limits.total_bytes || edges > limits.edges {
                return Err(BundleError::new(ErrorCode::Limit, "expanded bytes or dependency edge budget exceeded"));
            }
            if !sorted_unique(&blob.dependencies) {
                return Err(invalid("dependencies must be unique and sorted"));
            }
            remaining[i] = blob.dependencies.len();
            for dependency in &blob.dependencies {
                crate::work::io(0)?;
                let j = index.get(dependency).ok_or_else(|| {
                    BundleError::new(ErrorCode::MissingDependency, format!("missing blob dependency {}", dependency.hex()))
                })?;
                dependents[*j].push(i);
            }
        }
        // Iterative Kahn traversal bounds stack usage even on a hostile deep DAG.
        let mut queue: VecDeque<_> = remaining.iter().enumerate().filter(|(_, n)| **n == 0).map(|(i, _)| i).collect();
        let mut depth = vec![1usize; self.blobs.len()];
        let mut visited = 0;
        while let Some(i) = queue.pop_front() {
            crate::work::io(0)?;
            visited += 1;
            if depth[i] > limits.dependency_depth {
                return Err(BundleError::new(ErrorCode::Limit, "dependency depth budget exceeded"));
            }
            for j in &dependents[i] {
                depth[*j] = depth[*j].max(depth[i] + 1);
                remaining[*j] -= 1;
                if remaining[*j] == 0 {
                    queue.push_back(*j);
                }
            }
        }
        if visited != self.blobs.len() {
            return Err(BundleError::new(ErrorCode::DependencyCycle, "content dependency graph contains a cycle"));
        }
        let mut frontier: Vec<_> = used.iter().copied().collect();
        while let Some(id) = frontier.pop() {
            crate::work::io(0)?;
            for dep in &self.blobs[index[&id]].dependencies {
                if used.insert(*dep) {
                    frontier.push(*dep);
                }
            }
        }
        if used.len() != self.blobs.len() {
            return Err(invalid("unreferenced artifact in inventory"));
        }
        Ok(InventorySummary {
            manifest_commitment: self.commitment()?,
            sessions: self.sessions.len(),
            blobs: self.blobs.len(),
            bytes: total,
            edges,
            dependency_depth: *depth.iter().max().unwrap_or(&0),
        })
    }
}
