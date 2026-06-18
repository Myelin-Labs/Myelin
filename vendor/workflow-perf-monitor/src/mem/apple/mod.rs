// Newer Apple SDKs no longer expose the malloc zone internals this crate's
// heap module binds against, but Spora only uses VM-based process memory info.
// Keep the supported `vm` API and skip compiling the stale heap wrapper.
pub mod vm;
