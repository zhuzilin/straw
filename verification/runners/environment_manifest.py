"""Validate environment contracts or export their declared review dependencies."""

import argparse
import hashlib
import json
import re
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "verification/environment_contracts.json"
STATUSES = {
    "explicit_model",
    "trusted_contract",
    "assumption",
    "assumption_in_model",
    "partial_model",
    "direct_unproved_premise",
    "measurement_only",
    "explicit_model_and_adapter_assumptions",
}


@lru_cache(maxsize=1)
def tla_sources():
    """Module names stay globally unique across feature directories."""
    sources = {}
    for path in sorted((ROOT / "verification").rglob("*.tla")):
        if path.stem in sources:
            raise ValueError(f"Duplicate TLA module: {path.stem}")
        sources[path.stem] = path
    return sources


def checked_path(path):
    if not isinstance(path, str) or Path(path).is_absolute() or ".." in Path(path).parts:
        raise ValueError(f"Invalid repository path: {path!r}")
    full = (ROOT / path).resolve()
    if not full.is_relative_to(ROOT) or not full.is_file():
        raise ValueError(f"Missing/unsafe repository file: {path}")
    return full


def unique_strings(value, label):
    if not isinstance(value, list) or any(not isinstance(s, str) or not s.strip() for s in value):
        raise ValueError(f"{label} must be a list of nonempty strings")
    if len(value) != len(set(value)):
        raise ValueError(f"Duplicate entry in {label}")
    return value


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate registry key: {key}")
        result[key] = value
    return result


def load_environment():
    registry = json.loads(MANIFEST.read_text(), object_pairs_hook=unique_object)
    if registry["version"] != 2:
        raise ValueError("Unknown environment registry version")
    contracts = registry["contracts"]
    for identity, contract in contracts.items():
        if not re.fullmatch(r"ENV-[A-Z]+(?:-[A-Z]+)*", identity):
            raise ValueError(f"Invalid environment contract ID: {identity}")
        if contract["status"] not in STATUSES:
            raise ValueError(f"Unknown environment status in {identity}")
        for label in ("layer", "statement"):
            if not isinstance(contract.get(label), str) or not contract[label].strip():
                raise ValueError(f"Missing {label} in {identity}")
        review = contract.get("review", {})
        for label in ("provided_by", "implementation_obligation", "unproved"):
            if not isinstance(review.get(label), str) or not review[label].strip():
                raise ValueError(f"Missing review.{label} in {identity}")
        for label in ("sources", "evidence"):
            for path in unique_strings(contract[label], f"{identity}.{label}"):
                checked_path(path)
    # An omitted consumer must not silently disappear from impact reports.
    sources = tla_sources()
    models = {name for name in sources if not name.startswith("Environment")}
    if set(registry["models"]) != models:
        raise ValueError("Environment registry must enumerate every TLA consumer")
    for name, spec in registry["models"].items():
        if not set(unique_strings(spec["contracts"], f"{name}.contracts")) <= contracts.keys():
            raise ValueError(f"Unknown environment contract in {name}")
        for path in unique_strings(spec["environment_modules"], f"{name}.environment_modules"):
            checked_path(path)
        source = sources[name].read_text()
        extended = re.search(r"(?m)^EXTENDS ([^\n]+)", source)
        if extended is None:
            raise ValueError(f"Missing supported EXTENDS declaration in {name}")
        imported = {s.strip() for s in extended[1].split(",") if s.strip().startswith("Environment")}
        declared = {Path(p).stem for p in spec["environment_modules"] if Path(p).suffix == ".tla"}
        if imported != declared:
            raise ValueError(f"Environment imports/registry disagree for {name}: {imported} != {declared}")
        if len(declared) != len(spec["environment_modules"]):
            raise ValueError(f"Unexpected environment module path in {name}")
    return registry


def environment_metadata(name, registry):
    spec = registry["models"][name]
    return {
        "environment_registry_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
        "environment_contracts": spec["contracts"],
        "environment_sha256": {
            path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in spec["environment_modules"]
        },
    }


def layer_index(registry, contracts=None):
    selected = set(registry["contracts"] if contracts is None else contracts)
    layers = {}
    for identity, contract in registry["contracts"].items():
        if identity in selected:
            layers.setdefault(contract["layer"], []).append(identity)
    return layers


def review_report(registry, contracts=None, models=None, layers=None):
    selected = set(contracts or registry["contracts"])
    if models:
        selected = {identity for name in models for identity in registry["models"][name]["contracts"]}
    elif layers:
        selected = {identity for identity, contract in registry["contracts"].items() if contract["layer"] in layers}
    consumers = {
        identity: [name for name, spec in registry["models"].items() if identity in spec["contracts"]]
        for identity in registry["contracts"]
        if identity in selected
    }
    affected = set(models) if models else {name for names in consumers.values() for name in names}
    return {
        "version": registry["version"],
        "environment_registry_sha256": hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
        "scope": "Declared dependencies, not a proof of semantic completeness or adapter conformance. Evidence links do not certify assumptions.",
        "layers": layer_index(registry, selected),
        "contracts": {
            identity: {**registry["contracts"][identity], "consumers": names} for identity, names in consumers.items()
        },
        "models": {
            name: {**spec, **environment_metadata(name, registry)}
            for name, spec in registry["models"].items()
            if name in affected
        },
        "not_modeled": registry["not_modeled"],
    }


def main():
    registry = load_environment()
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--check", action="store_true", help="Validate inventory and paths without a prover")
    selection.add_argument("--list-layers", action="store_true", help="List component layers and their contract IDs")
    selection.add_argument(
        "--layer",
        action="append",
        choices=layer_index(registry),
        help="Review all contracts in selected component layers",
    )
    selection.add_argument(
        "--contract", action="append", choices=registry["contracts"], help="Review selected contracts"
    )
    selection.add_argument(
        "--model", action="append", choices=registry["models"], help="Review a consumer's assumptions"
    )
    args = parser.parse_args()
    if args.check:
        print(f"Validated {len(registry['contracts'])} contracts and {len(registry['models'])} consumers")
    elif args.list_layers:
        print(json.dumps(layer_index(registry), indent=2))
    else:
        print(json.dumps(review_report(registry, args.contract, args.model, args.layer), indent=2))


if __name__ == "__main__":
    main()
