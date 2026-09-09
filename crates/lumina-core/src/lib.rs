//! LuminaShelf high-performance core.
//!
//! This bootstrap commit intentionally lands the network/provider/download core
//! first. The Tauri shell, SQLite state layer and Android packaging are added in
//! follow-up commits so `main` always contains a coherent buildable slice.

pub mod download;
pub mod library;
pub mod mirror;
pub mod network;
pub mod provider;

pub use library::{LibraryFormat, LibraryItem};
pub use mirror::{MirrorEndpoint, MirrorRegistry, MirrorSourceKind, MirrorState, ProbeMetrics, ScoreWeights, SourceCatalog, SourceDefinition};
pub use network::{AppResolver, ReqwestResolver, ResolveResult, ResolverMode, ResolverPolicy};
pub use provider::{BookDetails, BookFormat, BookSummary, GutendexProvider, ProviderDescriptor, ProviderRegistry, SearchQuery, SearchResult, ZLibraryHistoryItem, ZLibraryHistoryPage, ZLibraryProfile, ZLibraryProvider, ZLibrarySession};
