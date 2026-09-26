import gleam/list
import gleam/option.{None, Some}
import gleam/string
import gleeunit
import gleeunit/should
import scintilla_lambda_pub_core.{
  type LambdaManifest, Amd64, Binary, Executable, LambdaManifest, Linux,
  UnsafeExecutableCommand,
}

pub fn main() -> Nil {
  gleeunit.main()
}

fn manifest(command: String) -> LambdaManifest {
  LambdaManifest(
    api_version: "scintilla.run/lambda/v1",
    name: "example-lambda",
    runtime: Binary,
    protocol: "stdio-json-v1",
    handler: "main",
    runtime_version: None,
    artifact: Executable(
      command: command,
      sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      os: Linux,
      architecture: Amd64,
      args: [],
    ),
  )
}

pub fn accepts_bare_executable_test() {
  scintilla_lambda_pub_core.validate(manifest("./bin/lambda"))
  |> should.be_ok
}

pub fn rejects_shell_command_test() {
  scintilla_lambda_pub_core.validate(manifest("sh -c ./lambda"))
  |> should.equal(Error(UnsafeExecutableCommand))
}

pub fn rejects_every_non_contract_command_character_test() {
  ["./lambda$HOME", "./lambda`id`", "./lambda\nnext", "./λ", "/", "./"]
  |> list.each(fn(command) {
    scintilla_lambda_pub_core.validate(manifest(command))
    |> should.equal(Error(UnsafeExecutableCommand))
  })
}

pub fn counts_unicode_characters_at_contract_boundary_test() {
  let original = manifest("./bin/lambda")
  let bounded =
    LambdaManifest(
      ..original,
      handler: string.repeat("🚀", 64),
      runtime_version: Some(string.repeat("🚀", 64)),
      artifact: Executable(
        command: "./bin/lambda",
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        os: Linux,
        architecture: Amd64,
        args: [string.repeat("🚀", 1024)],
      ),
    )
  scintilla_lambda_pub_core.validate(bounded)
  |> should.be_ok
}

pub fn exports_context_abi_and_lambda_descriptor_test() {
  scintilla_lambda_pub_core.context_abi
  |> should.equal("scintilla.run/context/v1")
  scintilla_lambda_pub_core.lambda_module_descriptor("run")
  |> should.equal(scintilla_lambda_pub_core.ModuleDescriptor(
    kind: scintilla_lambda_pub_core.LambdaModule,
    export_name: "run",
    context_abi: "scintilla.run/context/v1",
  ))
}

pub fn validates_module_descriptor_contract_test() {
  let valid = scintilla_lambda_pub_core.lambda_module_descriptor("run")
  scintilla_lambda_pub_core.validate_module_descriptor(valid)
  |> should.be_ok

  let stale =
    scintilla_lambda_pub_core.ModuleDescriptor(
      kind: scintilla_lambda_pub_core.LambdaModule,
      export_name: "run",
      context_abi: "scintilla.run/context/v0",
    )
  scintilla_lambda_pub_core.validate_module_descriptor(stale)
  |> should.equal(Error(scintilla_lambda_pub_core.UnsupportedContextAbi))
}

pub fn application_context_preserves_invocation_test() {
  let invocation = scintilla_lambda_pub_core.InvocationContext(
    abi: scintilla_lambda_pub_core.context_abi,
    invocation_id: "gleam-app",
    timeout_ms: 750,
    traceparent: None,
  )
  let context =
    scintilla_lambda_pub_core.application_context(invocation, "catalog")
  scintilla_lambda_pub_core.context_invocation(context)
  |> should.equal(invocation)
  scintilla_lambda_pub_core.context_state(context)
  |> should.equal("catalog")
}
