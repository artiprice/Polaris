use polaris_code_intel::{Lang, ParserPool, RepoMapBudget, SymbolGraph, repo_map, skeleton};
use polaris_sdk::ids::TokenCount;
use std::path::PathBuf;
use std::time::Instant;
use sysinfo::{Pid, ProcessesToUpdate, System};

fn resident_bytes() -> Option<u64> {
    let mut system = System::new();
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}

#[test]
#[ignore = "release-mode workload; run explicitly to record machine-specific budgets"]
fn repo_map_scalability_and_skeleton_throughput() -> Result<(), Box<dyn std::error::Error>> {
    let pool = ParserPool::new();
    let mut graph = SymbolGraph::new();
    for file_count in [1_000usize, 10_000, 100_000] {
        let started = Instant::now();
        for index in graph.file_count()..file_count {
            let source = format!("pub fn symbol_{index}() {{}}\n");
            let parsed = pool.parse(Lang::Rust, source.as_bytes(), None)?;
            graph.add_file(PathBuf::from(format!("src/module_{index}.rs")), &parsed)?;
        }
        let build_elapsed = started.elapsed();
        let map_started = Instant::now();
        let map = repo_map(
            &graph,
            &RepoMapBudget {
                max_tokens: TokenCount::new(u32::MAX),
                focus_files: Vec::new(),
            },
        )?;
        let map_elapsed = map_started.elapsed();
        assert_eq!(map.entries.len(), file_count);
        assert_eq!(graph.file_count(), file_count);
        eprintln!(
            "repo-map: files={file_count} build_elapsed_ms={} map_elapsed_ms={} entries={} estimated_tokens={}",
            build_elapsed.as_millis(),
            map_elapsed.as_millis(),
            map.entries.len(),
            map.token_estimate.get()
        );
    }

    let source = "pub fn load(x: u32) -> u32 { x + 1 }\n".repeat(30_000);
    let parsed = pool.parse(Lang::Rust, source.as_bytes(), None)?;
    let started = Instant::now();
    let outline = skeleton(&parsed)?;
    let elapsed = started.elapsed();
    let source_bytes = u64::try_from(source.len())?;
    let elapsed_ns = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX).max(1);
    let bytes_per_second = source_bytes
        .saturating_mul(1_000_000_000)
        .checked_div(elapsed_ns)
        .unwrap_or(0);
    let mib_per_second = bytes_per_second / (1024 * 1024);
    eprintln!(
        "skeleton-throughput: bytes={} items={} elapsed_ms={} mib_per_second={mib_per_second}",
        source.len(),
        outline.items.len(),
        elapsed.as_millis()
    );
    let rss = resident_bytes();
    eprintln!("code-intel-rss-after-scalability-bytes={rss:?}");
    if let Some(bytes) = rss {
        assert!(bytes < 2 * 1024 * 1024 * 1024);
    }
    Ok(())
}
