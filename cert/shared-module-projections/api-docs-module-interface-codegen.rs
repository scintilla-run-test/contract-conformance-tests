#![allow(clippy::needless_return)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleInterfaceLanguage {
    #[serde(rename = "rust")]
    Rust,
    #[serde(rename = "typescript")]
    TypeScript,
    #[serde(rename = "dart")]
    Dart,
    #[serde(rename = "erlang")]
    Erlang,
    #[serde(rename = "gleam")]
    Gleam,
    #[serde(rename = "sml")]
    StandardMl,
    #[serde(rename = "ocaml")]
    Ocaml,
    #[serde(rename = "racket")]
    Racket,
    #[serde(rename = "ada")]
    Ada,
    #[serde(rename = "modula2")]
    Modula2,
    #[serde(rename = "modula3")]
    Modula3,
    #[serde(rename = "haskell")]
    Haskell,
    #[serde(rename = "wit")]
    Wit,
}

impl ModuleInterfaceLanguage {
    pub const ALL: [Self; 13] = [
        Self::Rust,
        Self::TypeScript,
        Self::Dart,
        Self::Erlang,
        Self::Gleam,
        Self::StandardMl,
        Self::Ocaml,
        Self::Racket,
        Self::Ada,
        Self::Modula2,
        Self::Modula3,
        Self::Haskell,
        Self::Wit,
    ];

    pub const fn as_str(self) -> &'static str {
        return match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Dart => "dart",
            Self::Erlang => "erlang",
            Self::Gleam => "gleam",
            Self::StandardMl => "sml",
            Self::Ocaml => "ocaml",
            Self::Racket => "racket",
            Self::Ada => "ada",
            Self::Modula2 => "modula2",
            Self::Modula3 => "modula3",
            Self::Haskell => "haskell",
            Self::Wit => "wit",
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleInterfaceRuntimeProfile {
    OresStack,
    Scintilla,
    BeamScale,
}

impl ModuleInterfaceRuntimeProfile {
    pub const fn as_str(self) -> &'static str {
        return match self {
            Self::OresStack => "ores_stack",
            Self::Scintilla => "scintilla",
            Self::BeamScale => "beam_scale",
        };
    }

    pub const fn allows_projection_language(self, language: ModuleInterfaceLanguage) -> bool {
        return match self {
            Self::OresStack | Self::Scintilla => true,
            Self::BeamScale => matches!(
                language,
                ModuleInterfaceLanguage::Erlang | ModuleInterfaceLanguage::Gleam
            ),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInterfaceSpec {
    pub module_name: String,
    pub operation_name: String,
    pub contract_id: String,
}

impl ModuleInterfaceSpec {
    pub fn new(
        module_name: impl Into<String>,
        operation_name: impl Into<String>,
        contract_id: impl Into<String>,
    ) -> Self {
        return Self {
            module_name: module_name.into(),
            operation_name: operation_name.into(),
            contract_id: contract_id.into(),
        };
    }

    fn validate(&self) -> Result<(), ModuleInterfaceCodegenError> {
        validate_identifier("module_name", &self.module_name)?;
        validate_identifier("operation_name", &self.operation_name)?;

        if self.contract_id.trim().is_empty() || self.contract_id.chars().any(char::is_whitespace) {
            return Err(ModuleInterfaceCodegenError::InvalidContractId);
        }

        return Ok(());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedModuleInterface {
    pub language: ModuleInterfaceLanguage,
    pub file_name: String,
    pub source: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ModuleInterfaceCodegenError {
    #[error("{field} must be a portable identifier: {value}")]
    InvalidIdentifier { field: &'static str, value: String },
    #[error("contract_id must be non-empty and contain no whitespace")]
    InvalidContractId,
    #[error("runtime profile {runtime_profile} does not admit {language} as a module-interface projection")]
    RuntimeLanguageMismatch {
        runtime_profile: &'static str,
        language: &'static str,
    },
}

pub fn render_module_interface(
    runtime_profile: ModuleInterfaceRuntimeProfile,
    language: ModuleInterfaceLanguage,
    spec: &ModuleInterfaceSpec,
) -> Result<GeneratedModuleInterface, ModuleInterfaceCodegenError> {
    spec.validate()?;

    if !runtime_profile.allows_projection_language(language) {
        return Err(ModuleInterfaceCodegenError::RuntimeLanguageMismatch {
            runtime_profile: runtime_profile.as_str(),
            language: language.as_str(),
        });
    }

    return Ok(GeneratedModuleInterface {
        language,
        file_name: file_name(language, spec),
        source: render_source(language, spec),
    });
}

pub fn render_module_interface_matrix(
    runtime_profile: ModuleInterfaceRuntimeProfile,
    languages: &[ModuleInterfaceLanguage],
    spec: &ModuleInterfaceSpec,
) -> Result<Vec<GeneratedModuleInterface>, ModuleInterfaceCodegenError> {
    let rendered = languages
        .iter()
        .copied()
        .map(|language| render_module_interface(runtime_profile, language, spec))
        .collect::<Result<Vec<_>, _>>()?;

    return Ok(rendered);
}

fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), ModuleInterfaceCodegenError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(ModuleInterfaceCodegenError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    };

    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(ModuleInterfaceCodegenError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    }

    return Ok(());
}

fn file_name(language: ModuleInterfaceLanguage, spec: &ModuleInterfaceSpec) -> String {
    let snake = to_snake_case(&spec.module_name);
    let pascal = to_pascal_case(&spec.module_name);

    return match language {
        ModuleInterfaceLanguage::Rust => format!("{snake}.rs"),
        ModuleInterfaceLanguage::TypeScript => format!("{snake}.ts"),
        ModuleInterfaceLanguage::Dart => format!("{snake}.dart"),
        ModuleInterfaceLanguage::Erlang => format!("{snake}.erl"),
        ModuleInterfaceLanguage::Gleam => format!("{snake}.gleam"),
        ModuleInterfaceLanguage::StandardMl => format!("{snake}.sml"),
        ModuleInterfaceLanguage::Ocaml => format!("{snake}.ml"),
        ModuleInterfaceLanguage::Racket => format!("{snake}.rkt"),
        ModuleInterfaceLanguage::Ada => format!("{snake}.ads"),
        ModuleInterfaceLanguage::Modula2 => format!("{pascal}.def"),
        ModuleInterfaceLanguage::Modula3 => format!("{pascal}.i3"),
        ModuleInterfaceLanguage::Haskell => format!("{pascal}.hsig"),
        ModuleInterfaceLanguage::Wit => format!("{snake}.wit"),
    };
}

fn render_source(language: ModuleInterfaceLanguage, spec: &ModuleInterfaceSpec) -> String {
    return match language {
        ModuleInterfaceLanguage::Rust => render_rust(spec),
        ModuleInterfaceLanguage::TypeScript => render_typescript(spec),
        ModuleInterfaceLanguage::Dart => render_dart(spec),
        ModuleInterfaceLanguage::Erlang => render_erlang(spec),
        ModuleInterfaceLanguage::Gleam => render_gleam(spec),
        ModuleInterfaceLanguage::StandardMl => render_sml(spec),
        ModuleInterfaceLanguage::Ocaml => render_ocaml(spec),
        ModuleInterfaceLanguage::Racket => render_racket(spec),
        ModuleInterfaceLanguage::Ada => render_ada(spec),
        ModuleInterfaceLanguage::Modula2 => render_modula2(spec),
        ModuleInterfaceLanguage::Modula3 => render_modula3(spec),
        ModuleInterfaceLanguage::Haskell => render_haskell(spec),
        ModuleInterfaceLanguage::Wit => render_wit(spec),
    };
}

fn line_header(spec: &ModuleInterfaceSpec, prefix: &str) -> String {
    return format!(
        "{prefix} @generated by ORESoftware/api-docs module-interface-codegen\n{prefix} contract: {}\n",
        spec.contract_id
    );
}

fn block_header(spec: &ModuleInterfaceSpec) -> String {
    return format!(
        "(* @generated by ORESoftware/api-docs module-interface-codegen *)\n(* contract: {} *)\n",
        spec.contract_id
    );
}

fn render_rust(spec: &ModuleInterfaceSpec) -> String {
    let type_name = to_pascal_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = line_header(spec, "//");

    return format!(
        "{header}\npub trait ModuleContract {{\n    type Context;\n    type Input;\n    type Output;\n    type Error;\n\n    fn {operation}(\n        &self,\n        context: &Self::Context,\n        input: Self::Input,\n    ) -> Result<Self::Output, Self::Error>;\n}}\n\npub struct {type_name};\n\nimpl ModuleContract for {type_name} {{\n    type Context = ();\n    type Input = String;\n    type Output = String;\n    type Error = String;\n\n    fn {operation}(\n        &self,\n        _context: &Self::Context,\n        input: Self::Input,\n    ) -> Result<Self::Output, Self::Error> {{\n        return Ok(input);\n    }}\n}}\n"
    );
}

fn render_typescript(spec: &ModuleInterfaceSpec) -> String {
    let operation = &spec.operation_name;
    let header = line_header(spec, "//");

    return format!(
        "{header}\nexport interface ModuleContract<Context, Input, Output> {{\n  readonly contract_id: string;\n  {operation}(context: Context, input: Input): Promise<Output>;\n}}\n\nexport const module_export = {{\n  contract_id: '{}',\n  async {operation}(_context: unknown, input: string): Promise<string> {{\n    return input;\n  }},\n}} satisfies ModuleContract<unknown, string, string>;\n",
        spec.contract_id
    );
}

fn render_dart(spec: &ModuleInterfaceSpec) -> String {
    let type_name = to_pascal_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = line_header(spec, "//");

    return format!(
        "{header}\nabstract interface class ModuleContract<Context, Input, Output> {{\n  String get contractId;\n  Future<Output> {operation}(Context context, Input input);\n}}\n\nfinal class {type_name} implements ModuleContract<Object?, String, String> {{\n  @override\n  String get contractId => '{}';\n\n  @override\n  Future<String> {operation}(Object? _context, String input) async {{\n    return input;\n  }}\n}}\n\nfinal moduleExport = {type_name}();\n",
        spec.contract_id
    );
}

fn render_erlang(spec: &ModuleInterfaceSpec) -> String {
    let module = to_snake_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = line_header(spec, "%");

    return format!(
        "{header}\n-module({module}).\n\n-export([{operation}/2]).\n\n-callback {operation}(Context :: term(), Input :: term()) ->\n    {{ok, Output :: term()}} | {{error, Reason :: term()}}.\n\n-spec {operation}(term(), term()) -> {{ok, term()}} | {{error, term()}}.\n{operation}(_Context, Input) ->\n    {{ok, Input}}.\n"
    );
}

fn render_gleam(spec: &ModuleInterfaceSpec) -> String {
    let operation = &spec.operation_name;
    let header = line_header(spec, "//");

    return format!(
        "{header}\npub opaque type Module(context, input, output) {{\n  Module({operation}: fn(context, input) -> Result(output, String))\n}}\n\npub fn module(\n  {operation}: fn(context, input) -> Result(output, String),\n) -> Module(context, input, output) {{\n  Module({operation}: {operation})\n}}\n\npub fn {operation}(\n  module_export: Module(context, input, output),\n  context: context,\n  input: input,\n) -> Result(output, String) {{\n  let Module(callback) = module_export\n  callback(context, input)\n}}\n"
    );
}

fn render_sml(spec: &ModuleInterfaceSpec) -> String {
    let signature = format!("{}_MODULE", to_screaming_snake_case(&spec.module_name));
    let structure = to_pascal_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = block_header(spec);

    return format!(
        "{header}\nsignature {signature} =\nsig\n  type context\n  type input\n  type output\n  val {operation} : context -> input -> output\nend\n\nstructure {structure} :> {signature} =\nstruct\n  type context = unit\n  type input = string\n  type output = string\n  fun {operation} _ input = input\nend\n"
    );
}

fn render_ocaml(spec: &ModuleInterfaceSpec) -> String {
    let module_type = format!("{}_MODULE", to_screaming_snake_case(&spec.module_name));
    let module_name = to_pascal_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = block_header(spec);

    return format!(
        "{header}\nmodule type {module_type} = sig\n  type context\n  type input\n  type output\n  val {operation} : context -> input -> output\nend\n\nmodule {module_name} : {module_type} = struct\n  type context = unit\n  type input = string\n  type output = string\n  let {operation} _context input = input\nend\n"
    );
}

fn render_racket(spec: &ModuleInterfaceSpec) -> String {
    let operation = &spec.operation_name;
    let header = line_header(spec, ";;");

    return format!(
        "{header}\n#lang racket\n\n(struct module-export ({operation}) #:transparent)\n\n(define module-export/c\n  (struct/c module-export (-> any/c any/c any/c)))\n\n(define example-module\n  (module-export (lambda (_context input) input)))\n\n(provide\n (contract-out\n  [example-module module-export/c]))\n"
    );
}

fn render_ada(spec: &ModuleInterfaceSpec) -> String {
    let package = to_ada_case(&spec.module_name);
    let operation = to_pascal_case(&spec.operation_name);
    let header = line_header(spec, "--");

    return format!(
        "{header}\npackage {package} is\n   type Context is private;\n   type Input is private;\n   type Output is private;\n\n   function {operation} (Ctx : Context; Value : Input) return Output;\n\nprivate\n   type Context is null record;\n   type Input is new Integer;\n   type Output is new Integer;\nend {package};\n"
    );
}

fn render_modula2(spec: &ModuleInterfaceSpec) -> String {
    let module = to_pascal_case(&spec.module_name);
    let operation = to_pascal_case(&spec.operation_name);
    let header = block_header(spec);

    return format!(
        "{header}\nDEFINITION MODULE {module};\n\nTYPE\n  Context;\n  Input;\n  Output;\n\nPROCEDURE {operation}(VAR Ctx: Context; VAR Value: Input; VAR Result: Output);\n\nEND {module}.\n"
    );
}

fn render_modula3(spec: &ModuleInterfaceSpec) -> String {
    let module = to_pascal_case(&spec.module_name);
    let operation = to_pascal_case(&spec.operation_name);
    let header = block_header(spec);

    return format!(
        "{header}\nINTERFACE {module};\n\nTYPE\n  Context = REFANY;\n  Input = REFANY;\n  Output = REFANY;\n\nPROCEDURE {operation}(ctx: Context; value: Input): Output;\n\nEND {module}.\n"
    );
}

fn render_haskell(spec: &ModuleInterfaceSpec) -> String {
    let module = to_pascal_case(&spec.module_name);
    let operation = &spec.operation_name;
    let header = line_header(spec, "--");

    return format!(
        "{header}\nsignature {module} where\n\ndata Context\ndata Input\ndata Output\n\n{operation} :: Context -> Input -> IO Output\n"
    );
}

fn render_wit(spec: &ModuleInterfaceSpec) -> String {
    let interface = to_kebab_case(&spec.module_name);
    let operation = to_kebab_case(&spec.operation_name);
    let header = line_header(spec, "//");

    return format!(
        "{header}\npackage ores:module-contract@1.0.0;\n\ninterface {interface} {{\n  record invocation-context {{\n    deadline-unix-ms: option<u64>,\n  }}\n\n  {operation}: func(input: list<u8>, context: invocation-context) -> result<list<u8>, string>;\n}}\n\nworld guest-module {{\n  export {interface};\n}}\n"
    );
}

fn split_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for character in value.chars() {
        if character == '_' || character == '-' {
            if !current.is_empty() {
                words.push(current.to_ascii_lowercase());
                current.clear();
            }
            continue;
        }

        if character.is_ascii_uppercase() && !current.is_empty() {
            words.push(current.to_ascii_lowercase());
            current.clear();
        }

        current.push(character);
    }

    if !current.is_empty() {
        words.push(current.to_ascii_lowercase());
    }

    return words;
}

fn to_snake_case(value: &str) -> String {
    return split_words(value).join("_");
}

fn to_kebab_case(value: &str) -> String {
    return split_words(value).join("-");
}

fn to_screaming_snake_case(value: &str) -> String {
    return split_words(value)
        .into_iter()
        .map(|word| word.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("_");
}

fn to_pascal_case(value: &str) -> String {
    let rendered = split_words(value)
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            return format!("{}{}", first.to_ascii_uppercase(), chars.as_str());
        })
        .collect::<String>();

    return rendered;
}

fn to_ada_case(value: &str) -> String {
    return split_words(value)
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            return format!("{}{}", first.to_ascii_uppercase(), chars.as_str());
        })
        .collect::<Vec<_>>()
        .join("_");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ModuleInterfaceSpec {
        return ModuleInterfaceSpec::new("catalog_worker", "handle", "ores-stack.worker.v1");
    }

    #[test]
    fn authored_wire_spellings_round_trip() {
        for language in ModuleInterfaceLanguage::ALL {
            let json = serde_json::to_string(&language).expect("language serializes");
            assert_eq!(json, format!("\"{}\"", language.as_str()));
            let decoded: ModuleInterfaceLanguage =
                serde_json::from_str(&json).expect("language deserializes");
            assert_eq!(decoded, language);
        }
    }

    #[test]
    fn matrix_is_deterministic_and_complete_for_scintilla() {
        let first = render_module_interface_matrix(
            ModuleInterfaceRuntimeProfile::Scintilla,
            &ModuleInterfaceLanguage::ALL,
            &spec(),
        )
        .expect("Scintilla should admit all projection languages");
        let second = render_module_interface_matrix(
            ModuleInterfaceRuntimeProfile::Scintilla,
            &ModuleInterfaceLanguage::ALL,
            &spec(),
        )
        .expect("second deterministic render should succeed");

        assert_eq!(first, second);
        assert_eq!(first.len(), 13);
        assert!(first
            .iter()
            .any(|artifact| artifact.language == ModuleInterfaceLanguage::Dart));
    }

    #[test]
    fn beamscale_fails_closed_for_non_beam_projections() {
        let error = render_module_interface(
            ModuleInterfaceRuntimeProfile::BeamScale,
            ModuleInterfaceLanguage::Rust,
            &spec(),
        )
        .expect_err("BeamScale must reject Rust as a guest projection");

        assert_eq!(
            error,
            ModuleInterfaceCodegenError::RuntimeLanguageMismatch {
                runtime_profile: "beam_scale",
                language: "rust",
            }
        );

        for language in [
            ModuleInterfaceLanguage::Erlang,
            ModuleInterfaceLanguage::Gleam,
        ] {
            render_module_interface(ModuleInterfaceRuntimeProfile::BeamScale, language, &spec())
                .expect("BEAM guest projections must remain admitted");
        }
    }

    #[test]
    fn native_module_families_render_native_contract_syntax() {
        let cases = [
            (
                ModuleInterfaceLanguage::StandardMl,
                "signature CATALOG_WORKER_MODULE",
            ),
            (
                ModuleInterfaceLanguage::Ocaml,
                "module type CATALOG_WORKER_MODULE",
            ),
            (ModuleInterfaceLanguage::Ada, "package Catalog_Worker is"),
            (
                ModuleInterfaceLanguage::Modula2,
                "DEFINITION MODULE CatalogWorker",
            ),
            (ModuleInterfaceLanguage::Modula3, "INTERFACE CatalogWorker"),
            (
                ModuleInterfaceLanguage::Haskell,
                "signature CatalogWorker where",
            ),
            (ModuleInterfaceLanguage::Racket, "contract-out"),
        ];

        for (language, expected) in cases {
            let generated = render_module_interface(
                ModuleInterfaceRuntimeProfile::Scintilla,
                language,
                &spec(),
            )
            .expect("native projection should render");
            assert!(generated.source.contains(expected));
        }
    }

    #[test]
    fn adapter_languages_render_typed_export_shapes() {
        let cases = [
            (ModuleInterfaceLanguage::Rust, "pub trait ModuleContract"),
            (
                ModuleInterfaceLanguage::TypeScript,
                "satisfies ModuleContract",
            ),
            (
                ModuleInterfaceLanguage::Dart,
                "abstract interface class ModuleContract",
            ),
            (ModuleInterfaceLanguage::Erlang, "-callback handle"),
            (ModuleInterfaceLanguage::Gleam, "pub opaque type Module"),
        ];

        for (language, expected) in cases {
            let generated = render_module_interface(
                ModuleInterfaceRuntimeProfile::Scintilla,
                language,
                &spec(),
            )
            .expect("adapter projection should render");
            assert!(generated.source.contains(expected));
        }
    }

    #[test]
    fn portable_identifier_validation_is_fail_closed() {
        let invalid = ModuleInterfaceSpec::new("../../module", "handle", "ores-stack.worker.v1");
        let error = render_module_interface(
            ModuleInterfaceRuntimeProfile::Scintilla,
            ModuleInterfaceLanguage::Rust,
            &invalid,
        )
        .expect_err("path-like module name must be rejected");

        assert!(matches!(
            error,
            ModuleInterfaceCodegenError::InvalidIdentifier {
                field: "module_name",
                ..
            }
        ));
    }
}
