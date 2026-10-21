//! Synthetic local measurement and fuzz seeds, not a production issuer or throughput guarantee.
use polaris_sdk::{contracts::Timestamp, ids::FrameId};
use polaris_security_capability::{
    Authority, Authorization, Clock, Facts, ResourcePath, ResourceScope, Right, Rights, Scope,
    Token, attenuate, authorize,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
struct FixtureClock;
impl Clock for FixtureClock {
    fn now(&self) -> Timestamp {
        Timestamp::UNIX_EPOCH
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let authority = Authority::new_session()?;
    let key = authority.public_key();
    let scope = Scope {
        resource: ResourceScope::Prefix(ResourcePath::new("src")?),
        max_churn_lines: 150,
        ttl: Duration::from_secs(60),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[Right::Read]),
    };
    let root = authority.issue_root(FrameId::from_u128(1), &scope, &FixtureClock)?;
    let resource = ResourcePath::new("src/a.rs")?;
    let facts = Facts {
        resource: &resource,
        symbol: None,
        churn_lines: 0,
        touches_unsafe: false,
        diff_blake3: None,
        right: Right::Read,
    };
    let corpus = Path::new("tests/fuzz/corpus/biscuit_token_bytes");
    std::fs::create_dir_all(corpus)?;
    println!(
        "synthetic fixture; samples=1000; debug_assertions={}",
        cfg!(debug_assertions)
    );
    for blocks in [1usize, 4, 16] {
        let mut token = root.clone();
        while token.block_count() < blocks {
            token = attenuate(&token, &scope, &FixtureClock)?;
        }
        let mut seed = key.as_bytes().to_vec();
        seed.extend_from_slice(token.to_bytes().as_bytes());
        std::fs::write(corpus.join(format!("generated-{blocks}-blocks")), seed)?;
        for parse_signatures in [false, true] {
            let mut samples = Vec::with_capacity(1000);
            for _ in 0..1000 {
                let started = Instant::now();
                let result = if parse_signatures {
                    let reparsed = Token::from_bytes(token.to_bytes().as_bytes(), &key)?;
                    authorize(&reparsed, &facts, &key, &FixtureClock)?
                } else {
                    authorize(&token, &facts, &key, &FixtureClock)?
                };
                if result != Authorization::Allowed {
                    return Err("synthetic scope unexpectedly rejected".into());
                }
                samples.push(started.elapsed().as_nanos());
            }
            samples.sort_unstable();
            let p99_index = samples
                .len()
                .saturating_mul(99)
                .div_ceil(100)
                .saturating_sub(1);
            let p99 = samples.get(p99_index).ok_or("missing p99")?;
            println!(
                "blocks={blocks} bytes={} parse_signatures={parse_signatures} p99_ns={p99}",
                token.to_bytes().len()
            );
        }
    }
    Ok(())
}
