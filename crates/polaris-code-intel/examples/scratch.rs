use tree_sitter::{Language, Parser};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let which = a.get(1).cloned().unwrap_or_default();
    let maxd: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    let src = std::fs::read_to_string(a.get(2).cloned().unwrap_or_default()).unwrap();
    let lang: Language = match which.as_str() {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "cuda" => tree_sitter_cuda::LANGUAGE.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut p = Parser::new();
    p.set_language(&lang).unwrap();
    let t = p.parse(&src, None).unwrap();
    println!(
        "== {which} abi={} err={}",
        lang.abi_version(),
        t.root_node().has_error()
    );
    let mut c = t.walk();
    loop {
        let n = c.node();
        let d = c.depth() as usize;
        if (n.is_named() || a.get(4).is_some()) && d <= maxd {
            let txt: String = src[n.byte_range()]
                .chars()
                .take(30)
                .collect::<String>()
                .replace('\n', " ");
            println!(
                "{}{}{} [{}]",
                "  ".repeat(d),
                c.field_name().map(|f| format!("{f}: ")).unwrap_or_default(),
                n.kind(),
                if n.child_count() == 0 {
                    txt
                } else {
                    String::new()
                }
            );
        }
        if d < maxd && c.goto_first_child() {
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return;
            }
        }
    }
}
