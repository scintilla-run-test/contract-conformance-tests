#!/usr/bin/env python3
import copy
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor" / "scintilla-interfaces"
schema = json.loads((VENDOR / "schemas/release-definition.schema.json").read_text())
fixture = json.loads((VENDOR / "fixtures/release-valid-flexible-middleware.json").read_text())

errors = list(Draft202012Validator(schema).iter_errors(fixture))
if errors:
    raise SystemExit("valid release fixture failed schema: " + "; ".join(e.message for e in errors))

if "apiContractSha256" not in schema["required"]:
    raise SystemExit("apiContractSha256 is not required by immutable release identity")
if fixture["apiContractSha256"] == fixture["routeTableSha256"]:
    raise SystemExit("API contract digest and route table digest are conflated")

missing = copy.deepcopy(fixture)
del missing["apiContractSha256"]
errors = list(Draft202012Validator(schema).iter_errors(missing))
if not errors:
    raise SystemExit("schema accepted release without apiContractSha256")

print("PASS exact Scintilla release contract requires distinct API contract identity")
