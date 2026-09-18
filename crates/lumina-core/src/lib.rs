//! LuminaShelf high-performance core.
//!
//! Network, provider, download, reading and persistent user state live here so UI
//! shells remain thin and replaceable.

pub mod backup;
pub mod download;
pub mod library;
pub mod mirror;
pub mod network;
pub mod provider;
pub mod reader;
pub mod storage;
pub mod user_state;

pub use library::{LibraryFormat, LibraryItem};
pub use mirror::{
    MirrorEndpoint, MirrorRegistry, MirrorRuntime, MirrorSourceKind, MirrorState, ProbeMetrics,
    ScoreWeights, SourceCatalog, SourceDefinition,
};
pub use network::{AppResolver, ReqwestResolver, ResolveResult, ResolverMode, ResolverPolicy};
pub use provider::{
    BookDetails, BookFormat, BookSummary, GutendexProvider, ProviderDescriptor, ProviderRegistry,
    SearchQuery, SearchResult, ZLibraryHistoryItem, ZLibraryHistoryPage, ZLibraryProfile,
    ZLibraryProvider, ZLibrarySession,
};
pub use reader::{
    open_book, open_book_chapter, open_book_metadata, ReaderBook, ReaderBookMetadata,
    ReaderChapter, ReaderChapterMetadata,
};
pub use storage::{PersistedDownloadTask, StateStore};
pub use user_state::{AccountProfile, Bookmark, FavoriteBook, ReadingProgress};
