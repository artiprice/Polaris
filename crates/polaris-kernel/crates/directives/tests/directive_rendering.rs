//! Directive Rendering (descriptor: specs/descriptors/directives_directive_rendering.json).

use polaris_kernel_directives::directive::{
    Arg, Budget, CiBackend, EnvKind, ListArg, ModelEffort, NvidiaDocsArg, NvidiaMode, Pool,
    SandboxMode, SessionDirective, StorageLevel, Toggle,
};
use polaris_kernel_directives::render::render;
use polaris_kernel_directives::{ParseLimits, parse};
use polaris_sdk::effort::Effort;
use proptest::prelude::*;

#[test]
fn render_canonical_forms() {
    assert_eq!(
        render(&SessionDirective::WebSearch(Toggle::On)),
        "$websearch"
    );
    assert_eq!(
        render(&SessionDirective::WebSearch(Toggle::Off)),
        "$websearch(off)"
    );
    assert_eq!(
        render(&SessionDirective::SubagentModels(vec![ModelEffort {
            model: "nano".into(),
            lo: Effort::Low,
            hi: Effort::Medium
        }])),
        "$subagent-models(nano(low-medium))"
    );
    assert_eq!(
        render(&SessionDirective::Custom {
            name: "x".into(),
            args: vec![Arg::Quoted(r#"a "q" \ b"#.into())]
        }),
        r#"$x("a \"q\" \\ b")"#
    );
    assert_eq!(
        render(&SessionDirective::Links(ListArg::Items(vec!["off".into()]))),
        r#"$links("off")"#
    );
    assert_eq!(
        render(&SessionDirective::Sandbox(SandboxMode::Auto)),
        "$sandbox(auto)"
    );
}

const KNOWN: &[&str] = &[
    "websearch",
    "subagents",
    "subagent-models",
    "links",
    "skills",
    "mcp",
    "rag",
    "nvidia",
    "nvidia-docs",
    "sandbox",
    "nebius-sandbox",
    "nebius-branching",
    "nebius-datalab",
    "nebius-speculative",
    "concurrency",
    "ratelimit",
    "budget",
    "storage-level",
    "browser",
    "needle",
    "env",
    "ci",
];

fn toggle() -> impl Strategy<Value = Toggle> {
    prop_oneof![Just(Toggle::On), Just(Toggle::Off)]
}

fn effort() -> impl Strategy<Value = Effort> {
    (1u8..=5).prop_filter_map("effort", |n| Effort::try_from(n).ok())
}

fn item() -> impl Strategy<Value = String> {
    prop_oneof!["[a-z0-9_./:-]{1,10}", "[a-zA-Z0-9 ,()'\"\\\\]{0,10}"]
}

fn list_arg() -> impl Strategy<Value = ListArg> {
    prop_oneof![
        Just(ListArg::On),
        Just(ListArg::Off),
        prop::collection::vec(item(), 1..4).prop_map(ListArg::Items)
    ]
}

fn leaf_arg() -> impl Strategy<Value = Arg> {
    prop_oneof![
        "[a-z0-9_./:-]{1,8}".prop_map(Arg::Atom),
        "[a-zA-Z0-9 ,=()'\"\\\\]{0,8}".prop_map(Arg::Quoted),
    ]
}

fn arg() -> impl Strategy<Value = Arg> {
    leaf_arg().prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            // The grammar has no kv-valued kv (`a=b=c`), so values are leaves or calls.
            (
                "[a-z_][a-z0-9_-]{0,6}",
                inner
                    .clone()
                    .prop_filter("kv value cannot be a kv", |a| !matches!(a, Arg::Kv { .. }))
            )
                .prop_map(|(key, value)| Arg::Kv {
                    key,
                    value: Box::new(value)
                }),
            (
                "[a-z_][a-z0-9_-]{0,6}",
                prop::collection::vec(inner.clone(), 0..3)
            )
                .prop_map(|(name, args)| Arg::Call {
                    name,
                    quoted: false,
                    args
                }),
            ("[A-Za-z0-9 /.]{0,8}", prop::collection::vec(inner, 0..3)).prop_map(|(name, args)| {
                Arg::Call {
                    name,
                    quoted: true,
                    args,
                }
            }),
        ]
    })
}

fn directive() -> impl Strategy<Value = SessionDirective> {
    let model = (
        "[a-z][a-z0-9.-]{0,6}(/[A-Za-z0-9._-]{1,8})?",
        effort(),
        effort(),
    )
        .prop_map(|(model, a, b)| ModelEffort {
            model,
            lo: a.min(b),
            hi: a.max(b),
        });
    let pool = prop_oneof![
        Just(Pool::Inference),
        Just(Pool::Web),
        Just(Pool::Docs),
        Just(Pool::Disk),
        Just(Pool::S1),
        Just(Pool::Sandbox)
    ];
    prop_oneof![
        toggle().prop_map(SessionDirective::WebSearch),
        toggle().prop_map(SessionDirective::Subagents),
        prop::collection::vec(model, 1..4).prop_map(SessionDirective::SubagentModels),
        list_arg().prop_map(SessionDirective::Links),
        list_arg().prop_map(SessionDirective::Skills),
        list_arg().prop_map(SessionDirective::Mcp),
        list_arg().prop_map(SessionDirective::Rag),
        prop_oneof![
            Just(NvidiaMode::Auto),
            Just(NvidiaMode::List),
            Just(NvidiaMode::Off)
        ]
        .prop_map(SessionDirective::Nvidia),
        Just(SessionDirective::NvidiaDocs(NvidiaDocsArg::Sync)),
        prop_oneof![
            Just(SandboxMode::Auto),
            Just(SandboxMode::Openshell),
            Just(SandboxMode::Microvm),
            Just(SandboxMode::Contree),
            Just(SandboxMode::Remote)
        ]
        .prop_map(SessionDirective::Sandbox),
        (any::<bool>(), proptest::option::of(any::<u8>()))
            .prop_map(|(on, max_branches)| SessionDirective::NebiusBranching { on, max_branches }),
        toggle().prop_map(SessionDirective::NebiusDatalab),
        toggle().prop_map(SessionDirective::NebiusSpeculative),
        prop::collection::vec((pool, any::<u32>()), 1..4).prop_map(SessionDirective::Concurrency),
        (
            proptest::option::of(any::<u32>()),
            proptest::option::of(any::<u32>())
        )
            .prop_map(|(rpm, tpm)| SessionDirective::RateLimit { rpm, tpm }),
        prop_oneof![
            "[0-9]{1,6}(\\.[0-9]{1,4})?".prop_map(|v| SessionDirective::Budget(Budget::Usd(v))),
            any::<u64>().prop_map(|v| SessionDirective::Budget(Budget::Tokens(v))),
        ],
        prop_oneof![Just(StorageLevel::Global), Just(StorageLevel::Isolated)]
            .prop_map(SessionDirective::StorageLevel),
        toggle().prop_map(SessionDirective::Browser),
        toggle().prop_map(SessionDirective::Needle),
        prop_oneof![
            Just(EnvKind::Nix),
            Just(EnvKind::Docker),
            Just(EnvKind::Host)
        ]
        .prop_map(SessionDirective::Env),
        Just(SessionDirective::Ci(CiBackend::Dagger)),
        ("[a-z][a-z0-9_-]{0,10}", prop::collection::vec(arg(), 0..4))
            .prop_filter(
                "custom name must not shadow a catalog directive",
                |(name, _)| !KNOWN.contains(&name.as_str())
            )
            .prop_map(|(name, args)| SessionDirective::Custom { name, args }),
    ]
}

proptest! {
    #[test]
    fn prop_render_roundtrip(d in directive()) {
        let text = render(&d);
        let parsed = parse(&text, &ParseLimits::default());
        prop_assert!(parsed.is_ok(), "{text:?} -> {parsed:?}");
        if let Ok(parsed) = parsed {
            let got: Vec<_> = parsed.directives.into_iter().map(|s| s.value).collect();
            prop_assert_eq!(got, vec![d], "rendered: {}", text);
        }
    }
}
