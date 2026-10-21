//! Descriptor: specs/descriptors/codeintel_skeleton_extraction.json

use polaris_code_intel::{
    CodeIntelError, Lang, MAX_SIGNATURE_BYTES, MAX_SKELETON_ITEMS, ParserPool, Skeleton,
    SymbolKind, skeleton,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn outline(lang: Lang, src: &str) -> Result<Skeleton, CodeIntelError> {
    let parsed = ParserPool::new().parse(lang, src.as_bytes(), None)?;
    skeleton(&parsed)
}

const RUST: &str = "\
use std::fmt;

pub struct Point { pub x: i32, pub y: i32 }

impl Point {
    pub fn new(x: i32, y: i32) -> Self { Point { x, y } }
    fn norm(&self) -> i32 {
        self.x * self.x + self.y * self.y
    }
}

pub trait Shape {
    fn area(&self) -> f64;
}

pub fn add(a: i32, b: i32) -> i32 {
    let hidden_body_marker = a + b;
    hidden_body_marker
}
";

#[test]
fn rust_fn_signature_kept_body_elided() -> TestResult {
    let sk = outline(Lang::Rust, RUST)?;
    let want = "\
pub struct Point
impl Point
    pub fn new(x: i32, y: i32) -> Self
    fn norm(&self) -> i32
pub trait Shape
    fn area(&self) -> f64
pub fn add(a: i32, b: i32) -> i32
";
    assert_eq!(sk.render(), want);
    assert!(!sk.render().contains("hidden_body_marker"));
    let add = sk.items.last().ok_or("no items")?;
    assert_eq!((add.kind, add.name.as_str()), (SymbolKind::Function, "add"));
    assert_eq!(add.depth, 0);
    let methods: Vec<_> = sk
        .items
        .iter()
        .filter(|i| i.kind == SymbolKind::Method)
        .map(|i| (i.name.as_str(), i.depth))
        .collect();
    assert_eq!(methods, [("new", 1), ("norm", 1), ("area", 1)]);
    Ok(())
}

#[test]
fn item_spans_cover_the_whole_declaration() -> TestResult {
    let sk = outline(Lang::Rust, RUST)?;
    let add = sk.items.last().ok_or("no items")?;
    let text = add
        .span
        .text(RUST.as_bytes())
        .ok_or("span not on a boundary")?;
    assert!(text.starts_with("pub fn add"));
    assert!(text.ends_with("}"));
    assert!(text.contains("hidden_body_marker"));
    Ok(())
}

#[test]
fn rust_misc_items_are_listed() -> TestResult {
    let src = "\
pub struct Tup(i32, i32);
struct Unit;
pub enum E { A, B }
mod inner { pub fn g() {} }
const K: u32 = 1;
static S: &str = \"x\";
type Al = Vec<u8>;
macro_rules! mm { () => {} }
pub async unsafe fn af<'a>(a: &'a str) -> Result<(), E> where E: Copy { loop {} }
impl std::fmt::Display for Unit { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) } }
";
    let sk = outline(Lang::Rust, src)?;
    let want = "\
pub struct Tup(i32, i32)
struct Unit
pub enum E
mod inner
    pub fn g()
const K: u32 = 1
static S: &str = \"x\"
type Al = Vec<u8>
macro_rules! mm
pub async unsafe fn af<'a>(a: &'a str) -> Result<(), E> where E: Copy
impl std::fmt::Display for Unit
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
";
    assert_eq!(sk.render(), want);
    Ok(())
}

#[test]
fn python_class_methods_listed() -> TestResult {
    let src = "\
import os

@register
class Greeter(Base):
    \"\"\"Doc string.\"\"\"
    def hello(self, name: str) -> str:
        return \"hi \" + name

    @staticmethod
    async def make(n: int = 3):
        pass

def top(a, b=1):
    return a
";
    let sk = outline(Lang::Python, src)?;
    let want = "\
class Greeter(Base)
    def hello(self, name: str) -> str
    async def make(n: int = 3)
def top(a, b=1)
";
    assert_eq!(sk.render(), want);
    let kinds: Vec<_> = sk.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [
            SymbolKind::Class,
            SymbolKind::Method,
            SymbolKind::Method,
            SymbolKind::Function
        ]
    );
    let class = sk.items.first().ok_or("no class")?;
    let text = class.span.text(src.as_bytes()).ok_or("bad span")?;
    assert!(text.starts_with("@register"), "span keeps decorators");
    Ok(())
}

#[test]
fn cuda_kernel_qualifiers_preserved() -> TestResult {
    let src = "\
__global__ void saxpy(float a, const float* x, float* y, int n) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n) y[i] = a * x[i] + y[i];
}
__device__ __forceinline__ float square(float v) { return v * v; }
template <typename T> __global__ void fill(T* p, T v) { p[0] = v; }
int main() { saxpy<<<1, 32>>>(1.0f, nullptr, nullptr, 0); return 0; }
";
    let sk = outline(Lang::Cuda, src)?;
    let want = "\
__global__ void saxpy(float a, const float* x, float* y, int n)
__device__ __forceinline__ float square(float v)
template <typename T> __global__ void fill(T* p, T v)
int main()
";
    assert_eq!(sk.render(), want);
    Ok(())
}

#[test]
fn cpp_namespaces_classes_and_prototypes() -> TestResult {
    let src = "\
#include <string>
namespace geo {
class Shape : public Base {
public:
    Shape();
    virtual ~Shape();
    virtual double area() const = 0;
    int id() const { return id_; }
private:
    int id_;
};
struct Pt { int x; };
}
int geo_count(int n) { return n; }
int proto(int);
int ns::Other::f(int a) { return a; }
";
    let sk = outline(Lang::Cpp, src)?;
    let want = "\
namespace geo
    class Shape : public Base
        Shape()
        virtual ~Shape()
        virtual double area() const = 0
        int id() const
    struct Pt
int geo_count(int n)
int proto(int)
int ns::Other::f(int a)
";
    assert_eq!(sk.render(), want);
    Ok(())
}

#[test]
fn typescript_declarations() -> TestResult {
    let src = "\
import { x } from \"y\";
export interface Named { name: string; greet(n: number): string }
export class Box<T> extends Base {
  private v: number = 1;
  constructor(v: number) { super(); }
  async get(id: T): Promise<void> { await 1; }
}
export function build(a: number, b?: string): Box<number> { return new Box(a); }
const arrow = (a: number): number => a + 1;
type Id = string;
enum Color { Red, Green }
namespace Util { export function q() {} }
";
    let sk = outline(Lang::TypeScript, src)?;
    let want = "\
export interface Named
    greet(n: number): string
export class Box<T> extends Base
    constructor(v: number)
    async get(id: T): Promise<void>
export function build(a: number, b?: string): Box<number>
const arrow = (a: number): number =>
type Id = string
enum Color
namespace Util
    export function q()
";
    assert_eq!(sk.render(), want);
    Ok(())
}

#[test]
fn syntax_error_file_still_yields_partial_skeleton() -> TestResult {
    let src = "fn good(a: i32) -> i32 { a }\nfn broken( {\npub struct Survivor;\nfn after() {}\n";
    let parsed = ParserPool::new().parse(Lang::Rust, src.as_bytes(), None)?;
    assert!(parsed.has_syntax_error());
    let sk = skeleton(&parsed)?;
    assert!(sk.items.iter().any(|i| i.name == "good"), "{}", sk.render());
    Ok(())
}

#[test]
fn multiline_signatures_collapse_to_one_line() -> TestResult {
    let src = "pub fn long(\n    a: i32,\n    b: i32,\n) -> i32 {\n    a + b\n}\n";
    let sk = outline(Lang::Rust, src)?;
    let item = sk.items.first().ok_or("no items")?;
    assert_eq!(item.signature, "pub fn long( a: i32, b: i32, ) -> i32");
    Ok(())
}

#[test]
fn oversized_signature_is_truncated_on_a_char_boundary() -> TestResult {
    let params = "\u{e9}".repeat(3_000);
    let src = format!("fn f({params}: i32) {{}}\n");
    let sk = outline(Lang::Rust, &src)?;
    let item = sk.items.first().ok_or("no items")?;
    assert!(item.signature.len() <= MAX_SIGNATURE_BYTES + 3);
    assert!(item.signature.ends_with("..."));
    Ok(())
}

#[test]
fn deeply_nested_modules_do_not_overflow_the_stack() -> TestResult {
    let depth = 3_000;
    let src = format!("{}{}", "mod m { ".repeat(depth), "}".repeat(depth));
    let sk = outline(Lang::Rust, &src)?;
    assert_eq!(sk.items.len(), depth);
    let deepest = sk.items.iter().map(|i| i.depth).max();
    assert_eq!(deepest, Some(u32::try_from(depth - 1)?));
    Ok(())
}

#[test]
fn item_count_limit_returns_too_large() -> TestResult {
    let src = "fn f(){}\n".repeat(MAX_SKELETON_ITEMS + 1);
    let err = outline(Lang::Rust, &src).err();
    assert_eq!(
        err,
        Some(CodeIntelError::TooLarge {
            limit: MAX_SKELETON_ITEMS
        })
    );
    Ok(())
}

#[test]
fn empty_source_has_an_empty_skeleton() -> TestResult {
    for lang in Lang::ALL {
        let sk = outline(lang, "")?;
        assert!(sk.items.is_empty());
        assert_eq!(sk.render(), "");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn skeleton_spans_are_in_bounds_and_ordered(names in proptest::collection::vec("[a-z]{1,8}", 0..12)) {
        let src: String = names.iter().enumerate().map(|(i, n)| format!("pub fn {n}{i}(x: u8) -> u8 {{ x }}\n")).collect();
        let sk = outline(Lang::Rust, &src).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(sk.items.len(), names.len());
        let mut last_end = 0usize;
        for item in &sk.items {
            prop_assert!(item.span.start >= last_end);
            prop_assert!(item.span.end <= src.len());
            prop_assert!(item.span.text(src.as_bytes()).is_some());
            last_end = item.span.end;
        }
    }

    #[test]
    fn bodies_never_leak_into_signatures(marker in "[a-z]{6,12}") {
        let src = format!("fn f() {{ let {marker}_x = 1; }}\nclass_like! {{ }}\n");
        let sk = outline(Lang::Rust, &src).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(!sk.render().contains(&marker));
    }
}
