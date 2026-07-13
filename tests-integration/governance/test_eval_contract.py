import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
ADR = REPO_ROOT / "docs/adr/0001-semantic-authority-time-privacy.md"
THREAT_MODEL = REPO_ROOT / "docs/security/threat-model-v1.md"
EVAL_ROOT = REPO_ROOT / "evals/v1"
MANIFEST = EVAL_ROOT / "manifest.json"
EVENT_SCHEMA = EVAL_ROOT / "contracts/event-schema-v1.json"
METRICS = EVAL_ROOT / "metrics.json"
EVAL_RUNNER = EVAL_ROOT / "run.py"
TASK_REPORT = REPO_ROOT / "docs/baseline/task-0.2-report.md"

CORPORA = {
    "stocks": EVAL_ROOT / "cases/stocks.jsonl",
    "projects": EVAL_ROOT / "cases/projects.jsonl",
    "knowledge": EVAL_ROOT / "cases/knowledge.jsonl",
    "adversarial": EVAL_ROOT / "cases/adversarial.jsonl",
}

LOCKED_PATHS = {
    "docs/adr/0001-semantic-authority-time-privacy.md",
    "docs/security/threat-model-v1.md",
    "evals/v1/contracts/event-schema-v1.json",
    "evals/v1/metrics.json",
    "evals/v1/run.py",
    "evals/v1/cases/stocks.jsonl",
    "evals/v1/cases/projects.jsonl",
    "evals/v1/cases/knowledge.jsonl",
    "evals/v1/cases/adversarial.jsonl",
}


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_jsonl(path: Path):
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line]
    return [json.loads(line) for line in lines]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_runner_module():
    spec = importlib.util.spec_from_file_location("eval_v1_runner", EVAL_RUNNER)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def mutated(value):
    if isinstance(value, bool):
        return not value
    if isinstance(value, int | float):
        return value + 1
    if isinstance(value, str):
        return f"{value}-MUTATED"
    if isinstance(value, list):
        return [*value, "MUTATED"]
    if isinstance(value, dict):
        return {**value, "MUTATED": True}
    raise AssertionError(f"No mutation strategy for {type(value).__name__}")


def test_required_governance_documents_are_decisive():
    adr = ADR.read_text(encoding="utf-8")
    threat_model = THREAT_MODEL.read_text(encoding="utf-8")

    for required in (
        "DECISION: Event Ledger is the sole semantic transition authority",
        "DECISION: UUIDv7",
        "DECISION: half-open valid-time intervals",
        "DECISION: Keycloak",
        "HARD GATE: remote MCP remains disabled",
        "latest-user-wins",
        "deny-first",
        "No dual source of truth",
    ):
        assert required in adr

    for required in (
        "Severity",
        "Owner",
        "Control",
        "Eval/Test IDs",
        "RUSTSEC-2026-0204",
        "adversarial-031",
        "adversarial-032",
        "adversarial-033",
        "adversarial-034",
    ):
        assert required in threat_model


def test_event_schema_v1_freezes_required_authority_fields():
    schema = load_json(EVENT_SCHEMA)
    assert schema["$schema"] == "https://json-schema.org/draft/2020-12/schema"
    assert schema["$id"].endswith("/event-schema-v1.json")
    assert schema["additionalProperties"] is False

    required = set(schema["required"])
    assert {
        "schema_version",
        "event_id",
        "owner_id",
        "event_seq",
        "event_type",
        "recorded_at",
        "actor_id",
        "client_id",
        "operation_id",
        "request_hash",
        "payload",
        "prior_event_hash",
        "event_hash",
        "purge_epoch",
    } <= required

    payload = schema["properties"]["payload"]
    assert "oneOf" in payload
    assert schema["properties"]["event_seq"]["minimum"] == 1
    assert schema["properties"]["purge_epoch"]["minimum"] == 0


def test_eval_manifest_hash_lock_and_case_inventory():
    manifest = load_json(MANIFEST)
    assert manifest["contract_version"] == "evals/v1"
    assert manifest["seed"] == 20260713
    assert manifest["runner"]["timezone"]["policy"] == "host-independent"
    assert manifest["runner"]["locale"]["policy"] == "host-independent"
    assert manifest["runner"]["network"]["runner_policy"] == "blocked"

    locked = {item["path"]: item for item in manifest["locked_artifacts"]}
    assert set(locked) == LOCKED_PATHS
    for relative_path, entry in locked.items():
        path = REPO_ROOT / relative_path
        assert entry["sha256"] == sha256(path)
        assert entry["bytes"] == path.stat().st_size

    all_ids = []
    loaded = {}
    for domain, path in CORPORA.items():
        cases = load_jsonl(path)
        loaded[domain] = cases
        assert len(cases) >= 30
        assert manifest["corpora"][domain]["count"] == len(cases)
        assert manifest["corpora"][domain]["sha256"] == sha256(path)
        for case in cases:
            assert case["domain"] == domain
            assert case["id"].startswith(f"{domain}-")
            assert case["category"]
            assert isinstance(case["input"], dict) and case["input"]
            assert case["expected"]["answer_mode"]
            assert case["expected"]["invariants"]
            all_ids.append(case["id"])

    assert len(all_ids) == len(set(all_ids))
    assert any(case["category"] == "future_valid" for case in loaded["stocks"])
    assert any(case["category"] == "external_contradiction" for case in loaded["stocks"])
    assert any(case["category"] == "latest_user_wins" for case in loaded["projects"])
    assert any(case["category"] == "latest_user_wins" for case in loaded["knowledge"])

    adversarial_categories = Counter(case["category"] for case in loaded["adversarial"])
    for category in (
        "prompt_injection",
        "secret_egress",
        "stored_xss",
        "ssrf_path_traversal",
        "auth_reauth",
        "retry_concurrency",
    ):
        assert adversarial_categories[category] >= 5


def test_metric_contract_has_hard_invariants_and_regression_budget():
    metrics = load_json(METRICS)
    assert metrics["hard_invariants_pass_rate"] == 1.0
    assert metrics["retrieval"]["recall_at_10"]["minimum"] == 0.90
    assert metrics["retrieval"]["ndcg_at_10"]["minimum"] == 0.80
    assert metrics["retrieval"]["maximum_absolute_regression"] == 0.02
    assert metrics["extraction"]["schema_valid_after_bounded_repair"]["minimum"] == 0.99
    assert metrics["extraction"]["evidence_span_exactness"]["minimum"] == 1.0
    assert metrics["extraction"]["supported_claim_precision"]["minimum"] == 0.95
    assert metrics["extraction"]["confirmed_unsupported_claims"]["maximum"] == 0
    assert metrics["formulas"]["recall_at_10"]
    assert metrics["formulas"]["ndcg_at_10"]
    assert metrics["critical_regression"]


def test_reference_runner_executes_actual_vs_expected_and_metrics():
    completed = subprocess.run(
        [sys.executable, str(EVAL_RUNNER), "--manifest", str(MANIFEST)],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr or completed.stdout
    report = json.loads(completed.stdout)
    assert report["passed"] is True
    assert report["case_count"] >= 124
    assert report["actual_vs_expected_passed"] == report["case_count"]
    assert report["metrics"]["hard_invariants_pass_rate"] == 1.0
    assert report["metrics"]["recall_at_10"] >= 0.90
    assert report["metrics"]["ndcg_at_10"] >= 0.80
    assert report["metrics"]["evidence_span_exactness"] == 1.0
    assert report["metrics"]["confirmed_unsupported_claims"] == 0
    assert report["thresholds_passed"] is True


def test_reference_runner_checks_every_expected_field_and_named_invariant():
    runner = load_runner_module()
    cases = [case for path in CORPORA.values() for case in load_jsonl(path)]
    invariant_names = set()
    invariant_examples = {}

    for case in cases:
        report = runner.evaluate_case(case)
        assert report["passed"], report
        expected_checks = {key for key in case["expected"] if key != "invariants"}
        assert set(report["expected_checks"]) == expected_checks
        assert set(report["invariant_checks"]) == set(case["expected"]["invariants"])
        assert all(report["invariant_checks"].values())
        invariant_names.update(case["expected"]["invariants"])
        for invariant in case["expected"]["invariants"]:
            invariant_examples.setdefault(invariant, case)

        for key in expected_checks:
            changed = copy.deepcopy(case)
            changed["expected"][key] = mutated(changed["expected"][key])
            mutation_report = runner.evaluate_case(changed)
            assert mutation_report["passed"] is False, (case["id"], key)
            assert mutation_report["expected_checks"][key] is False

    assert invariant_names == set(load_json(METRICS)["hard_invariants"])
    for invariant in invariant_names:
        assert invariant in runner.INVARIANT_EVALUATORS
        unsafe = runner.Decision("unsafe_side_effect", {"event_count": 99})
        assert runner.evaluate_invariant(invariant, invariant_examples[invariant], unsafe) is False


def test_reference_runner_metrics_use_declared_denominators_and_regression_budget():
    runner = load_runner_module()
    contract = load_json(METRICS)
    synthetic = [
        {
            "passed": True,
            "hard_invariant_case": True,
            "invariants_passed": True,
            "schema_valid": True,
            "repair_attempts": 0,
            "spans": [True],
            "supported_claims_emitted": [True, False],
            "confirmed_unsupported_claims": 0,
            "retrieval": {"relevant": ["a", "b"], "ranked": ["a", "x", "b"]},
        },
        {
            "passed": False,
            "hard_invariant_case": True,
            "invariants_passed": False,
            "schema_valid": False,
            "repair_attempts": 3,
            "spans": [False],
            "supported_claims_emitted": [True],
            "confirmed_unsupported_claims": 1,
            "retrieval": {"relevant": ["c"], "ranked": ["x"]},
        },
    ]
    measured = runner.compute_metrics(synthetic, contract)
    assert measured["hard_invariants_pass_rate"] == 0.5
    assert measured["schema_valid_after_bounded_repair"] == 0.5
    assert measured["evidence_span_exactness"] == 0.5
    assert measured["supported_claim_precision"] == 2 / 3
    assert measured["confirmed_unsupported_claims"] == 1
    assert measured["recall_at_10"] == 0.5

    passing = {key: 1.0 for key in ("hard_invariants_pass_rate", "recall_at_10", "ndcg_at_10", "schema_valid_after_bounded_repair", "evidence_span_exactness", "supported_claim_precision")}
    passing["confirmed_unsupported_claims"] = 0
    baseline = {"recall_at_10": 1.0, "ndcg_at_10": 1.0}
    assert runner.evaluate_thresholds(passing, contract, baseline)[0] is True
    passing["ndcg_at_10"] = 0.97
    threshold_passed, failures = runner.evaluate_thresholds(passing, contract, baseline)
    assert threshold_passed is False
    assert any("maximum_absolute_regression" in failure for failure in failures)


def test_strict_environment_is_truthful_and_report_hashes_match():
    completed = subprocess.run(
        [sys.executable, str(EVAL_RUNNER), "--manifest", str(MANIFEST), "--strict-environment"],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr or completed.stdout
    report = json.loads(completed.stdout)
    environment = report["environment"]
    assert environment["passed"] is True
    assert environment["checks"] == {
        "locale_policy": True,
        "network_guard": True,
        "python_version": True,
        "timezone_policy": True,
        "uv_version": True,
    }
    assert environment["observed"]["preferred_encoding"]
    assert environment["observed"]["timezone"]

    task_report = TASK_REPORT.read_text(encoding="utf-8")
    manifest = load_json(MANIFEST)
    for corpus in manifest["corpora"].values():
        assert corpus["sha256"] in task_report
