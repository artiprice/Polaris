//! Dispatch contract for Arc, OneOf2, and OneOf3 content stores.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    ids::Cid,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn byte_count(length: usize) -> Result<u64, ContentStoreError> {
    u64::try_from(length).map_err(|_| ContentStoreError::Unavailable)
}

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Put(Vec<u8>),
    Contains(Cid),
    Get(Cid, u64),
}

struct FixtureStore {
    marker: u8,
    payload: Vec<u8>,
    calls: Arc<Mutex<Vec<Call>>>,
    fail: bool,
}

impl FixtureStore {
    fn new(marker: u8, calls: Arc<Mutex<Vec<Call>>>, fail: bool) -> Self {
        Self {
            marker,
            payload: b"fixture content payload".to_vec(),
            calls,
            fail,
        }
    }

    fn record(&self, call: Call) -> Result<(), ContentStoreError> {
        self.calls
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?
            .push(call);
        Ok(())
    }
}

impl ContentStore for FixtureStore {
    fn put(&self, content: &[u8]) -> Result<Cid, ContentStoreError> {
        self.record(Call::Put(content.to_vec()))?;
        if self.fail {
            return Err(ContentStoreError::TooLarge {
                limit: 4,
                size: byte_count(content.len())?,
            });
        }
        Ok(Cid::from_bytes([self.marker; 32]))
    }

    fn contains(&self, cid: &Cid) -> Result<bool, ContentStoreError> {
        self.record(Call::Contains(*cid))?;
        if self.fail {
            return Err(ContentStoreError::Corrupt(*cid));
        }
        Ok(cid.as_bytes() == &[self.marker; 32])
    }

    fn get(&self, cid: &Cid, max_bytes: u64) -> Result<Bytes, ContentStoreError> {
        self.record(Call::Get(*cid, max_bytes))?;
        if self.fail {
            return Err(ContentStoreError::NotFound(*cid));
        }
        let size = byte_count(self.payload.len())?;
        if size > max_bytes {
            return Err(ContentStoreError::TooLarge {
                limit: max_bytes,
                size,
            });
        }
        Ok(Bytes::from(self.payload.clone()))
    }
}

fn verify_dispatch<S: ContentStore>(
    marker: u8,
    make: impl Fn(bool, Arc<Mutex<Vec<Call>>>) -> S,
) -> TestResult {
    let content = b"written fixture bytes";
    let content_size = byte_count(content.len())?;
    let success_calls = Arc::new(Mutex::new(Vec::new()));
    let success_store = make(false, Arc::clone(&success_calls));
    let cid = success_store.put(content)?;
    assert_eq!(cid, Cid::from_bytes([marker; 32]));
    assert!(success_store.contains(&cid)?);
    assert_eq!(
        success_store.get(&cid, 64)?,
        Bytes::from_static(b"fixture content payload")
    );
    assert!(matches!(
        success_store.get(&cid, 3),
        Err(ContentStoreError::TooLarge { limit: 3, size: 23 })
    ));
    assert_eq!(
        *success_calls
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?,
        vec![
            Call::Put(content.to_vec()),
            Call::Contains(cid),
            Call::Get(cid, 64),
            Call::Get(cid, 3),
        ]
    );

    let error_calls = Arc::new(Mutex::new(Vec::new()));
    let error_store = make(true, Arc::clone(&error_calls));
    assert_eq!(
        error_store.put(content),
        Err(ContentStoreError::TooLarge {
            limit: 4,
            size: content_size,
        })
    );
    assert_eq!(
        error_store.contains(&cid),
        Err(ContentStoreError::Corrupt(cid))
    );
    assert_eq!(
        error_store.get(&cid, 64),
        Err(ContentStoreError::NotFound(cid))
    );
    assert_eq!(
        *error_calls
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?,
        vec![
            Call::Put(content.to_vec()),
            Call::Contains(cid),
            Call::Get(cid, 64),
        ]
    );
    Ok(())
}

#[test]
fn arc_content_store_forwards_arguments_and_errors() -> TestResult {
    verify_dispatch(1, |fail, calls| Arc::new(FixtureStore::new(1, calls, fail)))
}

#[test]
fn one_of_two_content_store_forwards_every_variant() -> TestResult {
    verify_dispatch(2, |fail, calls| {
        polaris_sdk::backends::OneOf2::<FixtureStore, FixtureStore>::A(FixtureStore::new(
            2, calls, fail,
        ))
    })?;
    verify_dispatch(3, |fail, calls| {
        polaris_sdk::backends::OneOf2::<FixtureStore, FixtureStore>::B(FixtureStore::new(
            3, calls, fail,
        ))
    })
}

#[test]
fn one_of_three_content_store_forwards_every_variant() -> TestResult {
    verify_dispatch(4, |fail, calls| {
        polaris_sdk::backends::OneOf3::<FixtureStore, FixtureStore, FixtureStore>::A(
            FixtureStore::new(4, calls, fail),
        )
    })?;
    verify_dispatch(5, |fail, calls| {
        polaris_sdk::backends::OneOf3::<FixtureStore, FixtureStore, FixtureStore>::B(
            FixtureStore::new(5, calls, fail),
        )
    })?;
    verify_dispatch(6, |fail, calls| {
        polaris_sdk::backends::OneOf3::<FixtureStore, FixtureStore, FixtureStore>::C(
            FixtureStore::new(6, calls, fail),
        )
    })
}
