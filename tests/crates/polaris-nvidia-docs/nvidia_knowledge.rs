//! `$nvidia(auto)` profiles, CUDA advisor rules and the documentation index.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use polaris_nvidia_docs::{
    ComputeCapability, DocIndex, DocTopic, NvidiaKnowledge, Severity, advise_cuda, auto_profile,
    chunk_markdown, html_to_markdown, parse_llms_txt, parse_nvidia_smi,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn profiles_find_stacks_and_target_architectures_with_evidence() -> TestResult {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir_all(workspace.path().join("src/kernels"))?;
    std::fs::create_dir_all(workspace.path().join("target/debug"))?;
    std::fs::write(
        workspace.path().join("src/kernels/add.cu"),
        "__global__ void add() {}\n",
    )?;
    std::fs::write(
        workspace.path().join("CMakeLists.txt"),
        "project(demo LANGUAGES CXX CUDA)\nset(CMAKE_CUDA_ARCHITECTURES 75;90)\n",
    )?;
    std::fs::write(
        workspace.path().join("requirements.txt"),
        "tensorrt_llm==0.20\nnumpy\n",
    )?;
    // Build output is never evidence.
    std::fs::write(workspace.path().join("target/debug/stale.ptx"), "// ptx")?;
    let profile = auto_profile(workspace.path(), "Why does my NCCL all-reduce hang?")?;
    let topics: Vec<&str> = profile.topics.iter().map(|topic| topic.as_str()).collect();
    assert_eq!(topics, ["cuda", "tensorrt-llm", "nccl"]);
    let archs: Vec<String> = profile
        .target_archs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(archs, ["sm_75", "sm_90"]);
    assert!(profile.evidence.contains(&"prompt: nccl".to_owned()));
    assert!(
        profile
            .evidence
            .iter()
            .any(|line| line == "src/kernels/add.cu: CUDA source"
                || line == "CMakeLists.txt: build uses CUDA"),
        "{:?}",
        profile.evidence
    );

    let gpus = parse_nvidia_smi("NVIDIA GeForce GTX 1650, 7.5\n");
    assert_eq!(gpus.len(), 1);
    assert_eq!(gpus[0].capability, ComputeCapability { major: 7, minor: 5 });
    let notes = profile.hardware_notes(&gpus);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].starts_with("the build targets sm_90, but the newest local GPU is sm_75"));
    Ok(())
}

const KERNELS: &str = r#"
#include <cuda_runtime.h>
// cudaMalloc(&ignored, 1); in a comment is not code
__global__ void scale(float* x, float s) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    x[i] = x[i] * s;
}

__global__ void safe(float* x, float s, int n) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i >= n) return;
    x[i] *= s;
}

__global__ void stride(const float* in, float* out, int n) {
    for (int i = blockIdx.x * blockDim.x + threadIdx.x; i < n; i += blockDim.x * gridDim.x) {
        out[i] = in[threadIdx.x * 32];
    }
}

__global__ void reduce(float* data) {
    __shared__ float tile[32][32];
    if (threadIdx.x < 16) {
        tile[threadIdx.x][0] = data[threadIdx.x];
        __syncthreads();
    }
}

void run(float* host, int n) {
    float* device;
    cudaMalloc(&device, n * sizeof(float));
    CUDA_CHECK(cudaMemcpy(device, host, n * sizeof(float), cudaMemcpyHostToDevice));
    scale<<<n / 256, 256>>>(device, 2.0f);
    safe<<<(n + 255) / 256, 256>>>(device, 2.0f, n);
    CUDA_CHECK(cudaGetLastError());
    printf("<<<not a launch>>>(x);");
}
"#;

#[test]
fn the_cuda_advisor_flags_the_classic_mistakes() {
    let findings = advise_cuda(KERNELS);
    let summary: Vec<(&str, usize)> = findings
        .iter()
        .map(|finding| (finding.rule, finding.line))
        .collect();
    assert_eq!(
        summary,
        [
            ("missing-grid-bounds", 5),
            ("strided-global-access", 17),
            ("shared-bank-conflict", 22),
            ("barrier-in-divergent-branch", 23),
            ("unchecked-api-call", 31),
        ],
        "{findings:#?}"
    );
    assert!(findings.iter().all(|finding| !finding.message.is_empty()));
    assert_eq!(
        findings
            .iter()
            .find(|finding| finding.rule == "shared-bank-conflict")
            .map(|finding| finding.severity),
        Some(Severity::Info)
    );
}

#[test]
fn an_unchecked_launch_is_reported() {
    let findings = advise_cuda(
        "__global__ void k(float* x) { x[0] = 1; }\nvoid go(float* x) {\n    k<<<1, 1>>>(x);\n    cudaDeviceSynchronize();\n}\n",
    );
    let rules: Vec<&str> = findings.iter().map(|finding| finding.rule).collect();
    assert_eq!(rules, ["unchecked-launch", "unchecked-api-call"]);
    assert_eq!(findings[0].line, 3);
    assert!(findings[0].message.contains("`k`"));
}

const LLMS: &str = "# CUDA Toolkit\n> Programming guides for CUDA.\n> Updated weekly.\n\n## Guides\n- [Programming Guide](https://docs.nvidia.com/cuda/cuda-c-programming-guide/index.md): kernels, memory\n- [Best Practices](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.md)\n\n## Optional\n* [PTX ISA](https://docs.nvidia.com/cuda/parallel-thread-execution/index.md): instruction set\n";

#[test]
fn llms_txt_links_keep_their_sections() {
    let parsed = parse_llms_txt(LLMS);
    assert_eq!(parsed.title, "CUDA Toolkit");
    assert_eq!(
        parsed.summary,
        "Programming guides for CUDA. Updated weekly."
    );
    let links: Vec<(&str, &str, &str)> = parsed
        .links
        .iter()
        .map(|link| {
            (
                link.section.as_str(),
                link.title.as_str(),
                link.description.as_str(),
            )
        })
        .collect();
    assert_eq!(
        links,
        [
            ("Guides", "Programming Guide", "kernels, memory"),
            ("Guides", "Best Practices", ""),
            ("Optional", "PTX ISA", "instruction set"),
        ]
    );
}

const GUIDE: &str = "# Memory\nGlobal memory is accessed through 32-byte transactions.\n\n## Coalescing\nWhen the threads of a warp access consecutive addresses, the loads are coalesced into few transactions.\n\n```cuda\n# not a heading inside code\nout[i] = in[i];\n```\n\n## Shared memory\nShared memory is divided into 32 banks; padding a tile avoids bank conflicts.\n";

#[test]
fn markdown_chunks_follow_headings_and_the_index_ranks_them() -> TestResult {
    let chunks = chunk_markdown(GUIDE, DocTopic::Cuda, "guide.md", 400)?;
    let headings: Vec<&str> = chunks.iter().map(|chunk| chunk.heading.as_str()).collect();
    assert_eq!(
        headings,
        ["Memory", "Memory › Coalescing", "Memory › Shared memory"]
    );
    assert!(chunks[1].text.contains("# not a heading inside code"));

    let mut index = DocIndex::default();
    for chunk in chunks {
        index.add(chunk)?;
    }
    let mut ptx = chunk_markdown(
        "# Registers\nPTX registers are virtual; ptxas allocates banks of physical registers.\n",
        DocTopic::Ptx,
        "ptx.md",
        400,
    )?;
    index.add(ptx.remove(0))?;
    let knowledge = NvidiaKnowledge::new(index);
    let hits = knowledge.query(None, "how do I avoid bank conflicts in shared memory", 3);
    assert_eq!(hits[0].chunk.heading, "Memory › Shared memory");
    let cuda_only = knowledge.query(Some(DocTopic::Cuda), "registers banks", 5);
    assert!(cuda_only.iter().all(|hit| hit.chunk.topic == "cuda"));
    assert!(
        knowledge
            .query(Some(DocTopic::Nccl), "anything", 5)
            .is_empty()
    );

    let mut seed = Vec::new();
    knowledge.index().write_seed(&mut seed)?;
    let reloaded = DocIndex::read_seed(std::io::Cursor::new(seed))?;
    assert_eq!(reloaded.len(), 4);
    assert!(DocIndex::read_seed(std::io::Cursor::new(b"{\"topic\":\"x\"}\n".to_vec())).is_err());
    Ok(())
}

#[test]
fn html_pages_become_markdown_without_scripts() -> TestResult {
    let markdown = html_to_markdown(
        "<html><head><script>track()</script></head><body><nav>menu</nav><h2>Streams</h2><p>Use <code>cudaStreamCreate</code> to overlap copies.</p></body></html>",
    )?;
    assert!(markdown.contains("## Streams"), "{markdown}");
    assert!(markdown.contains("`cudaStreamCreate`"), "{markdown}");
    assert!(
        !markdown.contains("track()") && !markdown.contains("menu"),
        "{markdown}"
    );
    Ok(())
}

/// A loopback docs server: the first request gets a page with an ETag, a request carrying that
/// ETag gets 304. Returns the port and the request heads it saw.
fn docs_server()
-> Result<(u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().take(3) {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
            let response = if head.contains("if-none-match: \"v1\"") {
                "HTTP/1.1 304 Not Modified\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_owned()
            } else {
                let body = "<html><body><h1>Streams</h1><p>Use cudaStreamCreate to overlap copies with kernels.</p></body></html>";
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\netag: \"v1\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            if let Ok(mut log) = log.lock() {
                log.push(head);
            }
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Ok((port, seen))
}

#[tokio::test]
async fn refreshes_are_conditional_and_spaced() -> TestResult {
    use polaris_nvidia_docs::DocFetcher;
    let (port, seen) = docs_server()?;
    let url = format!("http://127.0.0.1:{port}/cuda/streams.html");
    let spacing = std::time::Duration::from_millis(150);
    let mut fetcher = DocFetcher::new(spacing)?.allowing_loopback();
    let started = std::time::Instant::now();
    let chunks = fetcher
        .ingest(&url, DocTopic::Cuda, 800)
        .await?
        .ok_or("first fetch has content")?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].heading, "Streams");
    assert!(chunks[0].text.contains("cudaStreamCreate"));
    assert_eq!(
        fetcher
            .validators()
            .get(&url)
            .and_then(|v| v.etag.as_deref()),
        Some("\"v1\"")
    );
    assert!(
        fetcher.ingest(&url, DocTopic::Cuda, 800).await?.is_none(),
        "304 means unchanged"
    );
    assert!(started.elapsed() >= spacing, "the second request waited");
    let heads = seen.lock().map_err(|_| "poisoned")?.clone();
    assert_eq!(heads.len(), 2);
    assert!(heads[1].contains("if-none-match: \"v1\""));

    let refused = DocFetcher::new(spacing)?
        .fetch("https://example.com/cuda.html")
        .await;
    assert!(refused.is_err());
    let plain_http = DocFetcher::new(spacing)?
        .fetch(&format!("http://127.0.0.1:{port}/x"))
        .await;
    assert!(plain_http.is_err(), "loopback needs the explicit opt-in");
    Ok(())
}
