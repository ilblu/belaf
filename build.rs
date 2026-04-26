use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=TARGET={}",
        env::var("TARGET").unwrap_or_default()
    );

    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .replace("rustc ", "");
    println!("cargo:rustc-env=RUSTC_VERSION={}", rustc);

    generate_api_client();
}

/// Generate Rust types + client stub from `api-spec/openapi.cli.json`.
///
/// The spec is the committed OpenAPI document produced by the github-app's
/// `bun run apps/api/scripts/generate-openapi.ts`. To pick up server-side
/// schema changes: copy the new `openapi.cli.json` into `belaf/api-spec/` and
/// rebuild — the generated module will reflect the new wire format and the
/// compiler will surface every drift as a type error.
fn generate_api_client() {
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("api-spec/openapi.cli.json");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let file = std::fs::File::open(&spec_path)
        .unwrap_or_else(|e| panic!("failed to open {}: {e}", spec_path.display()));
    let spec: openapiv3::OpenAPI = serde_json::from_reader(file)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", spec_path.display()));

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .expect("progenitor failed to generate tokens");
    let ast = syn::parse2(tokens).expect("failed to parse generated TokenStream");
    let content = prettyplease::unparse(&ast);

    let out =
        Path::new(&env::var("OUT_DIR").expect("OUT_DIR not set")).join("belaf_api_codegen.rs");
    std::fs::write(&out, content)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", out.display()));
}
