//! Composition-root bundle (PLAN ADR-002) and generic enum dispatch without `dyn`.
//!
//! The kernel is generic over one [`Backends`] implementation. Runtime-selected variants (e.g. an
//! OpenShell, Contree or remote sandbox) are wrapped in [`OneOf2`] / [`OneOf3`], which forward every
//! registry trait to the active variant. The concrete wrappers that name L2 adapter types live in
//! the composition root (`polaris-kernel`), not here.

use core::future::Future;
use core::time::Duration;

use bytes::Bytes;

use crate::contracts::{Command, EventRecord, Unverified};
use crate::ids::{Cid, Seq};
use crate::registry::{
    CacheScope, CacheStoreError, ChatChunk, ChatChunkSource, ChatModel, ChatRequest,
    CommandReceipt, DecisionEngine, DecisionError, DocParser, DocSource, Document,
    EmbeddingProvider, EventSource, EventStore, EventStoreError, ExactCacheStore, ExecOutput,
    ExecRequest, FunctionalOperator, IngestError, KernelApi, KernelError, MemoryPressure,
    ModelError, PlatformError, PlatformOps, ProcessExit, ProcessTree, RedactStats, Redactor,
    SandboxBackend, SandboxError, SandboxHandle, SandboxSpec, SearchError, SearchHit, SearchQuery,
    SpawnSpec, StopOutcome, VectorChunk, VectorError, VectorHit, VectorStore, WebSearch,
};
use crate::system_one::{Decision, DecisionInput, Question};

/// Every backend the kernel needs, chosen once at the composition root.
pub trait Backends {
    /// System-Two chat model.
    type Chat: ChatModel;
    /// Embedding provider.
    type Embed: EmbeddingProvider;
    /// System-One decision engine.
    type Decide: DecisionEngine;
    /// Sandbox backend.
    type Sandbox: SandboxBackend;
    /// Web search.
    type Search: WebSearch;
    /// Document parser.
    type Docs: DocParser;
    /// Vector store.
    type Vectors: VectorStore;
    /// Event store.
    type Events: EventStore;
    /// OS seam.
    type Platform: PlatformOps;
    /// Secret / PII redactor.
    type Redact: Redactor;
    /// L2 exact cache store.
    type Cache: ExactCacheStore;
}

/// One of two backends, chosen at runtime; forwards every registry trait to the active variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneOf2<A, B> {
    /// First variant.
    A(A),
    /// Second variant.
    B(B),
}

/// One of three backends, chosen at runtime; forwards every registry trait to the active variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OneOf3<A, B, C> {
    /// First variant.
    A(A),
    /// Second variant.
    B(B),
    /// Third variant.
    C(C),
}

/// Applies `$body` to the active variant of a `OneOf2`, binding it as `$v`.
macro_rules! each2 {
    ($self:expr, $v:ident => $body:expr) => {
        match $self {
            OneOf2::A($v) => $body,
            OneOf2::B($v) => $body,
        }
    };
}

/// Applies `$body` to the active variant of a `OneOf3`, binding it as `$v`.
macro_rules! each3 {
    ($self:expr, $v:ident => $body:expr) => {
        match $self {
            OneOf3::A($v) => $body,
            OneOf3::B($v) => $body,
            OneOf3::C($v) => $body,
        }
    };
}

/// Implements the synchronous and asynchronous forwarding impls for one wrapper.
macro_rules! forward_all {
    ($wrapper:ident < $($p:ident),+ >, $each:ident) => {
        impl<$($p: ChatChunkSource),+> ChatChunkSource for $wrapper<$($p),+> {
            fn next_chunk(&mut self) -> impl Future<Output = Option<Result<ChatChunk, ModelError>>> + Send {
                async move { $each!(self, v => v.next_chunk().await) }
            }
        }

        impl<$($p: ChatModel),+> ChatModel for $wrapper<$($p),+> {
            type Stream = $wrapper<$($p::Stream),+>;

            fn stream(&self, req: ChatRequest) -> impl Future<Output = Result<Self::Stream, ModelError>> + Send {
                async move {
                    match self {
                        $($wrapper::$p(v) => v.stream(req).await.map($wrapper::$p),)+
                    }
                }
            }
        }

        impl<$($p: EmbeddingProvider),+> EmbeddingProvider for $wrapper<$($p),+> {
            fn dimension(&self) -> usize {
                $each!(self, v => v.dimension())
            }

            fn embed(&self, batch: &[&str]) -> impl Future<Output = Result<Vec<Vec<f32>>, ModelError>> + Send {
                async move { $each!(self, v => v.embed(batch).await) }
            }
        }

        impl<$($p: DecisionEngine),+> DecisionEngine for $wrapper<$($p),+> {
            fn decide(&self, schema: &[Question], input: &DecisionInput) -> impl Future<Output = Result<Decision, DecisionError>> + Send {
                async move { $each!(self, v => v.decide(schema, input).await) }
            }
        }

        impl<$($p: SandboxBackend),+> SandboxBackend for $wrapper<$($p),+> {
            fn create(&self, spec: &SandboxSpec) -> impl Future<Output = Result<SandboxHandle, SandboxError>> + Send {
                async move { $each!(self, v => v.create(spec).await) }
            }

            fn exec(&self, sandbox: &SandboxHandle, req: &ExecRequest) -> impl Future<Output = Result<ExecOutput, SandboxError>> + Send {
                async move { $each!(self, v => v.exec(sandbox, req).await) }
            }

            fn upload(&self, sandbox: &SandboxHandle, path: &str, bytes: Bytes) -> impl Future<Output = Result<(), SandboxError>> + Send {
                async move { $each!(self, v => v.upload(sandbox, path, bytes).await) }
            }

            fn download(&self, sandbox: &SandboxHandle, path: &str) -> impl Future<Output = Result<Bytes, SandboxError>> + Send {
                async move { $each!(self, v => v.download(sandbox, path).await) }
            }

            fn list_files(&self, sandbox: &SandboxHandle, dir: &str) -> impl Future<Output = Result<Vec<String>, SandboxError>> + Send {
                async move { $each!(self, v => v.list_files(sandbox, dir).await) }
            }

            fn destroy(&self, sandbox: &SandboxHandle) -> impl Future<Output = Result<(), SandboxError>> + Send {
                async move { $each!(self, v => v.destroy(sandbox).await) }
            }
        }

        impl<$($p: WebSearch),+> WebSearch for $wrapper<$($p),+> {
            fn search(&self, query: &SearchQuery) -> impl Future<Output = Result<Vec<SearchHit>, SearchError>> + Send {
                async move { $each!(self, v => v.search(query).await) }
            }
        }

        impl<$($p: DocParser),+> DocParser for $wrapper<$($p),+> {
            fn parse(&self, src: DocSource<'_>) -> Result<Document, IngestError> {
                $each!(self, v => v.parse(src))
            }
        }

        impl<$($p: VectorStore),+> VectorStore for $wrapper<$($p),+> {
            fn upsert(&self, chunks: &[VectorChunk]) -> Result<usize, VectorError> {
                $each!(self, v => v.upsert(chunks))
            }

            fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>, VectorError> {
                $each!(self, v => v.search(query, k))
            }

            fn delete(&self, ids: &[Cid]) -> Result<usize, VectorError> {
                $each!(self, v => v.delete(ids))
            }

            fn count(&self) -> Result<u64, VectorError> {
                $each!(self, v => v.count())
            }
        }

        impl<$($p: EventStore),+> EventStore for $wrapper<$($p),+> {
            fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
                $each!(self, v => v.append(record))
            }

            fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
                $each!(self, v => v.read_range(from, to))
            }
        }

        impl<$($p: EventSource),+> EventSource for $wrapper<$($p),+> {
            fn next_event(&mut self) -> impl Future<Output = Option<EventRecord>> + Send {
                async move { $each!(self, v => v.next_event().await) }
            }
        }

        impl<$($p: KernelApi),+> KernelApi for $wrapper<$($p),+> {
            type Events = $wrapper<$($p::Events),+>;

            fn submit(&self, cmd: Command<Unverified>) -> impl Future<Output = Result<CommandReceipt, KernelError>> + Send {
                async move { $each!(self, v => v.submit(cmd).await) }
            }

            fn subscribe(&self, from: Seq) -> impl Future<Output = Result<Self::Events, KernelError>> + Send {
                async move {
                    match self {
                        $($wrapper::$p(v) => v.subscribe(from).await.map($wrapper::$p),)+
                    }
                }
            }
        }

        impl<$($p: PlatformOps),+> PlatformOps for $wrapper<$($p),+> {
            fn spawn_tree(&self, spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
                $each!(self, v => v.spawn_tree(spec))
            }

            fn graceful_stop(&self, tree: &ProcessTree, grace: Duration) -> impl Future<Output = Result<StopOutcome, PlatformError>> + Send {
                async move { $each!(self, v => v.graceful_stop(tree, grace).await) }
            }

            fn kill_tree(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
                $each!(self, v => v.kill_tree(tree))
            }

            fn try_wait_tree(&self, tree: &ProcessTree) -> impl Future<Output = Result<Option<ProcessExit>, PlatformError>> + Send {
                async move { $each!(self, v => v.try_wait_tree(tree).await) }
            }

            fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
                $each!(self, v => v.memory_pressure())
            }
        }

        impl<$($p: Redactor),+> Redactor for $wrapper<$($p),+> {
            fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
                $each!(self, v => v.redact_into(input, out))
            }
        }

        impl<$($p: ExactCacheStore),+> ExactCacheStore for $wrapper<$($p),+> {
            fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
                $each!(self, v => v.get(key))
            }

            fn put(&self, key: [u8; 32], value: &[u8], scope: CacheScope) -> Result<(), CacheStoreError> {
                $each!(self, v => v.put(key, value, scope))
            }

            fn clear_scope(&self, scope: &CacheScope) -> Result<u64, CacheStoreError> {
                $each!(self, v => v.clear_scope(scope))
            }
        }
    };
}

forward_all!(OneOf2<A, B>, each2);
forward_all!(OneOf3<A, B, C>, each3);

impl<A, B> FunctionalOperator for OneOf2<A, B>
where
    A: FunctionalOperator,
    B: FunctionalOperator<In = A::In, Out = A::Out, Err = A::Err>,
{
    type In = A::In;
    type Out = A::Out;
    type Err = A::Err;

    fn apply(&self, input: Self::In) -> impl Future<Output = Result<Self::Out, Self::Err>> + Send {
        async move { each2!(self, v => v.apply(input).await) }
    }
}

impl<A, B, C> FunctionalOperator for OneOf3<A, B, C>
where
    A: FunctionalOperator,
    B: FunctionalOperator<In = A::In, Out = A::Out, Err = A::Err>,
    C: FunctionalOperator<In = A::In, Out = A::Out, Err = A::Err>,
{
    type In = A::In;
    type Out = A::Out;
    type Err = A::Err;

    fn apply(&self, input: Self::In) -> impl Future<Output = Result<Self::Out, Self::Err>> + Send {
        async move { each3!(self, v => v.apply(input).await) }
    }
}

/// Shared ownership: a store or redactor behind an `Arc` is still one (e.g. the kernel shares one
/// event store between the bus and replay).
mod shared {
    use std::sync::Arc;

    use super::{
        CacheScope, CacheStoreError, Cid, DocParser, DocSource, Document, EventRecord, EventStore,
        EventStoreError, ExactCacheStore, IngestError, RedactStats, Redactor, Seq, VectorChunk,
        VectorError, VectorHit, VectorStore,
    };

    impl<T: EventStore + ?Sized> EventStore for Arc<T> {
        fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
            (**self).append(record)
        }

        fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
            (**self).read_range(from, to)
        }
    }

    impl<T: VectorStore + ?Sized> VectorStore for Arc<T> {
        fn upsert(&self, chunks: &[VectorChunk]) -> Result<usize, VectorError> {
            (**self).upsert(chunks)
        }

        fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>, VectorError> {
            (**self).search(query, k)
        }

        fn delete(&self, ids: &[Cid]) -> Result<usize, VectorError> {
            (**self).delete(ids)
        }

        fn count(&self) -> Result<u64, VectorError> {
            (**self).count()
        }
    }

    impl<T: ExactCacheStore + ?Sized> ExactCacheStore for Arc<T> {
        fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
            (**self).get(key)
        }

        fn put(
            &self,
            key: [u8; 32],
            value: &[u8],
            scope: CacheScope,
        ) -> Result<(), CacheStoreError> {
            (**self).put(key, value, scope)
        }

        fn clear_scope(&self, scope: &CacheScope) -> Result<u64, CacheStoreError> {
            (**self).clear_scope(scope)
        }
    }

    impl<T: Redactor + ?Sized> Redactor for Arc<T> {
        fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
            (**self).redact_into(input, out)
        }
    }

    impl<T: DocParser + ?Sized> DocParser for Arc<T> {
        fn parse(&self, src: DocSource<'_>) -> Result<Document, IngestError> {
            (**self).parse(src)
        }
    }
}
