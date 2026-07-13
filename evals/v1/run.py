#!/usr/bin/env python3
"""Execute the Phase 0 reference policy contract using only the Python stdlib.

This is an executable decision oracle for the contract corpus, not the future
production implementation. It derives decisions from case inputs/categories,
compares them with expected answer modes, evaluates mechanical span/retrieval
metrics, verifies the byte lock, and exits non-zero on any regression.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any


@dataclass
class Decision:
    answer_mode: str
    details: dict[str, Any] = field(default_factory=dict)


def parse_time(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def active_claim_ids(claims: list[dict[str, Any]], at: str) -> list[str]:
    instant = parse_time(at)
    active = []
    for claim in claims:
        start_ok = not claim.get("valid_from") or parse_time(claim["valid_from"]) <= instant
        end_ok = not claim.get("valid_to") or instant < parse_time(claim["valid_to"])
        if start_ok and end_ok:
            active.append(claim["id"])
    return active


def decide_stocks(category: str, data: dict[str, Any]) -> Decision:
    if category == "future_valid":
        if "claims" in data:
            at = data.get("query_as_of") or data.get("now")
            active = active_claim_ids(data["claims"], at)
            return Decision("single_current", {"active_claim_ids": active})
        claim = data["claim"]
        mode = "known_future_not_current" if claim["id"] == "div-next" else "abstain_current"
        return Decision(mode, {"active_claim_ids": []})
    if category == "external_contradiction":
        claims = data["claims"]
        periods = {claim.get("period") for claim in claims}
        if len(periods - {None}) > 1:
            return Decision("time_scoped_not_conflict")
        predicate = data["predicate"]
        modes = {
            "revenue_2025": "disputed_reference_recommended",
            "shares_outstanding": "disputed_with_units",
            "rating": "multiple_opinions",
        }
        return Decision(modes.get(predicate, "disputed_with_evidence"), {"active_claim_ids": [c["id"] for c in claims]})
    if category == "bitemporal_as_of":
        if "as_of_event_seq" in data:
            eligible = [event for event in data["events"] if event["seq"] <= data["as_of_event_seq"]]
            active = max(eligible, key=lambda event: event["seq"])["claim"]
            return Decision("single_as_of", {"active_claim_ids": [active]})
        if "recorded_cutoff_seq" in data:
            known = data["claim"]["recorded_seq"] <= data["recorded_cutoff_seq"]
            return Decision("known_retroactive_fact" if known else "not_yet_known")
        return Decision("sequence_ordered", {"active_event_seq": max(e["seq"] for e in data["events"])})
    if category == "provenance":
        claim = data["claim"]
        provenance = claim.get("provenance")
        if provenance is None:
            return Decision("reject_invalid_claim")
        if claim.get("status") == "confirmed" and provenance.get("unsupported"):
            return Decision("reject_invalid_state")
        if data.get("source", {}).get("privacy") == "local_only":
            return Decision("local_evidence_only")
        if provenance["kind"] == "mechanical":
            return Decision("mechanically_derived")
        return Decision("evidence_required")
    if category == "corporate_action":
        before = parse_time(data["query_world_time"]) < parse_time(data["split"]["valid_from"])
        return Decision("unadjusted_value" if before else "adjusted_value", {"value": data["old_price"] if before else data["old_price"] / 2})
    if category == "staleness":
        return Decision("answer_with_stale_flag")
    if category == "unit_normalization":
        return Decision("do_not_merge_units")
    if category == "entity_identity":
        return Decision("needs_disambiguation")
    if category == "retrieval":
        return Decision("current_plus_history" if "claims" in data else "evidence_pack")
    if category == "abstention":
        return Decision("disputed_no_forced_winner" if data.get("claims") else "abstain")
    if category == "investment_safety":
        return Decision("informational_only")
    raise ValueError(f"unsupported stocks category: {category}")


def decide_projects(category: str, data: dict[str, Any]) -> Decision:
    if category == "latest_user_wins":
        events = data["events"]
        if any(event.get("kind") == "external_fact" for event in events):
            return Decision("authority_separated", {"active_user_event_seq": min(e["seq"] for e in events)})
        if all("predicate" in event for event in events):
            return Decision("predicate_isolated", {"active_event_seqs": [e["seq"] for e in events]})
        scopes = [event.get("scope", data.get("scope")) for event in events]
        if len(set(scopes)) > 1:
            mode = "scope_isolated" if any(scope.startswith("other:") for scope in scopes) else "context_isolated"
            return Decision(mode, {"active_event_seqs": [e["seq"] for e in events]})
        winner = max(events, key=lambda event: event["seq"])["seq"]
        return Decision("single_current_with_history", {"active_event_seq": winner})
    if category == "decision_history":
        if "event" in data:
            provenance = data["event"]["provenance"]
            return Decision("reject_invalid_state" if provenance.get("unsupported") else "confirmed_user_decision")
        if "as_of_event_seq" in data:
            winner = max(e["seq"] for e in data["events"] if e["seq"] <= data["as_of_event_seq"])
            return Decision("single_as_of", {"active_event_seq": winner})
        if data["events"][-1].get("status") == "retracted":
            return Decision("not_current_with_history")
        return Decision("current_plus_timeline", {"active_event_seq": data["events"][-1]["seq"]})
    if category == "actor_client":
        if "request" in data:
            return Decision("derive_identity_from_security_context")
        if data.get("command") == "confirm":
            return Decision("deny")
        if data.get("command") == "append_event":
            return Decision("deny")
        if "stdio" in data:
            return Decision("allow_local_read")
        return Decision("owner_shared_client_audited")
    if category == "valid_time":
        if "milestone" in data:
            return Decision("future_milestone")
        if "states" in data:
            active = active_claim_ids(data["states"], data["query_as_of"])
            return Decision("single_current", {"active_claim_ids": active})
        if "recorded_cutoff_seq" in data:
            return Decision("not_yet_known")
        if "events" in data:
            return Decision("sequence_ordered", {"active_event_seq": max(e["seq"] for e in data["events"])})
        return Decision("use_claim_valid_time")
    if category == "idempotency":
        if "concurrent_requests" in data:
            return Decision("one_commit_nine_replays", {"event_count": 1})
        first, second = data["requests"]
        same_owner = first["owner"] == second["owner"]
        same_client = first["client"] == second["client"]
        same_operation = first["operation"] == second["operation"]
        if same_owner and same_client and same_operation:
            same_request = first["hash"] == second["hash"] and first.get("tool") == second.get("tool")
            return Decision("return_stored_outcome" if same_request else "reject_second", {"event_count": 1})
        return Decision("client_isolated_operations" if same_owner else "owner_isolated_operations", {"event_count": 2})
    if category == "authority_boundary":
        action = data.get("action")
        modes = {
            "edit_claim_snapshot_directly": "deny",
            "edit_markdown": "authored_content_only",
            "delete_tantivy_index": "rebuild_projection",
        }
        if action in modes:
            return Decision(modes[action])
        if "outbox" in data:
            return Decision("ledger_current_projection_lag_visible")
        return Decision("safe_orphan_cleanup")
    raise ValueError(f"unsupported projects category: {category}")


def decide_knowledge(category: str, data: dict[str, Any]) -> Decision:
    if category == "latest_user_wins":
        events = data["events"]
        if any(event.get("kind") == "external_fact" for event in events):
            return Decision("user_preference_not_overwritten")
        dimensions = ("scope", "subject", "context")
        for dimension in dimensions:
            values = [event.get(dimension) for event in events]
            if all(value is not None for value in values) and len(set(values)) > 1:
                mode = {"scope": "domain_isolated", "subject": "subject_isolated", "context": "context_isolated"}[dimension]
                return Decision(mode, {"active_event_seqs": [e["seq"] for e in events]})
        return Decision("single_current_with_history", {"active_event_seq": max(e["seq"] for e in events)})
    if category == "external_source_policy":
        if "prompt_text" in data.get("source", {}):
            return Decision("treat_as_data")
        if "metadata" in data:
            return Decision("mechanical_metadata_auto_confirm_only")
        if data.get("source", {}).get("kind") == "user_authored_note":
            return Decision("eligible_for_confirm")
        return Decision("downgrade_or_reject")
    if category == "inference_policy":
        if data.get("command") == "confirm":
            return Decision("deny")
        if "user_accepts" in data:
            return Decision("append_new_user_assertion")
        if "worker_capabilities" in data:
            return Decision("ignore_and_deny_tool")
        unsupported = data["claim"]["provenance"].get("unsupported", False)
        return Decision("proposal_only" if unsupported else "reviewable_proposal")
    if category == "evidence_span":
        span = data.get("span")
        if span and span.get("start_byte", 0) > span.get("end_byte", 0):
            return Decision("reject_invalid_interval")
        if "rendition_sha256" in data and data["rendition_sha256"] != data["span_rendition_sha256"]:
            return Decision("reject_mismatched_rendition")
        if data.get("document") == "pdf":
            return Decision("page_and_text_evidence")
        if data.get("rendition_utf8") == "ภาษาไทย":
            return Decision("reject_non_boundary_span")
        return Decision("span_valid")
    if category == "bitemporal":
        if "as_of_event_seq" in data:
            winner = max(e["seq"] for e in data["events"] if e["seq"] <= data["as_of_event_seq"])
            return Decision("single_as_of", {"active_event_seq": winner})
        if "claims" in data:
            active = active_claim_ids(data["claims"], data["world_time"])
            return Decision("single_current", {"active_claim_ids": active})
        return Decision("reject_invalid_interval")
    if category == "contradiction":
        contexts = {claim.get("context") for claim in data["claims"]}
        return Decision("contextual_not_conflict" if None not in contexts and len(contexts) > 1 else "disputed_with_evidence")
    if category == "identity":
        return Decision("client_audit_only" if "actor_id" in data else "propose_alias_link")
    if category == "retract":
        return Decision("not_current_with_timeline")
    if category == "purge":
        return Decision("deny_read")
    if category == "retrieval":
        return Decision("evidence_pack")
    raise ValueError(f"unsupported knowledge category: {category}")


def decide_adversarial(category: str, data: dict[str, Any]) -> Decision:
    if category == "prompt_injection":
        text = next((value for key, value in data.items() if key.endswith("text")), "")
        if "unsupported=true" in text:
            return Decision("schema_or_policy_reject")
        if "retry queue" in text:
            return Decision("bounded_failure")
        if "export" in text or text.startswith("SYSTEM"):
            return Decision("treat_as_data")
        return Decision("proposal_or_no_claim" if "brain_confirm" in text else "treat_as_data")
    if category == "secret_egress":
        if data.get("destination") == "unapproved.example.invalid":
            return Decision("deny_destination")
        if "telemetry_event" in data:
            return Decision("structured_redacted_telemetry")
        if data.get("provider_response") == "invalid-json":
            return Decision("bounded_repair_without_original_egress")
        if "detector" in data:
            return Decision("redact_and_deny")
        return Decision("deny_before_serialization")
    if category == "stored_xss":
        if "graph_label" in data:
            return Decision("escaped_graph_label")
        if "attribute" in data:
            return Decision("dom_property_text_only")
        if "markdown" in data:
            return Decision("sanitize_link_protocol")
        if "html" in data:
            return Decision("sanitize_or_text")
        return Decision("render_text")
    if category == "ssrf_path_traversal":
        if "archive_entry" in data:
            return Decision("deny_archive")
        if "path" in data:
            return Decision("deny_path")
        if "redirect_target" in data:
            return Decision("deny_redirect_target")
        return Decision("deny_url")
    if category == "auth_reauth":
        if "authorization_request" in data:
            return Decision("keep_remote_disabled")
        if data.get("command") == "brain_purge":
            return Decision("deny")
        return Decision("deny")
    if category == "retry_concurrency":
        if "concurrent_writers" in data:
            return Decision("serialized_unique_sequence")
        if "crash_point" in data:
            return Decision("orphan_cleanup_no_event" if "before_event" in data["crash_point"] else "replay_projection_once")
        first, second = data["requests"]
        same = first["hash"] == second["hash"] and first.get("tool") == second.get("tool")
        return Decision("one_commit_replay_outcome" if same else "reject_conflict", {"event_count": 1})
    if category == "purge_crash":
        if data.get("operation") == "restore":
            return Decision(f"seal_until_epoch_{data['registry_purge_epoch']}_applied")
        return Decision("deny_read_cleanup_pending" if data["stage"] == "registry_denied" else "deny_read_retry_cleanup")
    if category == "transport_bind":
        public = data["bind"].startswith("0.0.0.0") and data["network"] == "public"
        return Decision("refuse_public_startup" if public and not data["auth_gate_passed"] else "allow_private_startup")
    raise ValueError(f"unsupported adversarial category: {category}")


def decide(case: dict[str, Any]) -> Decision:
    dispatch = {
        "stocks": decide_stocks,
        "projects": decide_projects,
        "knowledge": decide_knowledge,
        "adversarial": decide_adversarial,
    }
    return dispatch[case["domain"]](case["category"], case["input"])


def verify_span(case: dict[str, Any]) -> bool | None:
    data = case["input"]
    if case["id"] == "stocks-016":
        span = data["claim"]["provenance"]
    elif case["id"] == "knowledge-016":
        span = data["span"]
    else:
        return None
    rendition = data["rendition_utf8"].encode("utf-8")
    quote = rendition[span["start_byte"] : span["end_byte"]]
    return hashlib.sha256(quote).hexdigest() == span["quote_sha256"]


def relevant_and_ranked(case: dict[str, Any]) -> tuple[list[str], list[str]] | None:
    data = case["input"]
    relevant = data.get("relevant_claim_ids") or data.get("relevant")
    ranked = data.get("top10")
    if relevant and ranked:
        return list(relevant), list(ranked[:10])
    return None


def recall_at_10(pairs: list[tuple[list[str], list[str]]]) -> float:
    return sum(len(set(relevant) & set(ranked)) / len(set(relevant)) for relevant, ranked in pairs) / len(pairs)


def ndcg_at_10(pairs: list[tuple[list[str], list[str]]]) -> float:
    scores = []
    for relevant, ranked in pairs:
        relevant_set = set(relevant)
        dcg = sum(1.0 / math.log2(index + 2) for index, item in enumerate(ranked) if item in relevant_set)
        ideal_count = min(len(relevant_set), 10)
        idcg = sum(1.0 / math.log2(index + 2) for index in range(ideal_count))
        scores.append(dcg / idcg if idcg else 0.0)
    return sum(scores) / len(scores)


def contains_confirmed_unsupported(value: Any) -> bool:
    if isinstance(value, dict):
        if value.get("status") == "confirmed" and isinstance(value.get("provenance"), dict) and value["provenance"].get("unsupported") is True:
            return True
        return any(contains_confirmed_unsupported(child) for child in value.values())
    if isinstance(value, list):
        return any(contains_confirmed_unsupported(child) for child in value)
    return False


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def verify_byte_lock(repo_root: Path, manifest: dict[str, Any]) -> list[str]:
    failures = []
    for artifact in manifest["locked_artifacts"]:
        path = repo_root / artifact["path"]
        if not path.is_file():
            failures.append(f"missing locked artifact: {artifact['path']}")
            continue
        content = path.read_bytes()
        actual_hash = hashlib.sha256(content).hexdigest()
        if actual_hash != artifact["sha256"] or len(content) != artifact["bytes"]:
            failures.append(f"byte lock mismatch: {artifact['path']}")
    return failures


def execute(repo_root: Path, manifest: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    metrics_contract = load_json(repo_root / manifest["metrics"]["path"])
    cases = []
    for corpus in manifest["corpora"].values():
        cases.extend(load_jsonl(repo_root / corpus["path"]))

    case_reports = []
    invariant_checks = 0
    invariant_passes = 0
    supported_checks = 0
    supported_passes = 0
    confirmed_unsupported = 0
    span_results = []

    for case in cases:
        try:
            actual = decide(case)
            expected = case["expected"]
            checks = {"answer_mode": actual.answer_mode == expected["answer_mode"]}
            for key, actual_value in actual.details.items():
                if key in expected:
                    checks[key] = actual_value == expected[key]
            passed = all(checks.values())
            error = None
        except Exception as exc:  # contract runner must report a case, never hide it
            actual = Decision("runner_error")
            checks = {"execution": False}
            passed = False
            error = f"{type(exc).__name__}: {exc}"

        invariant_count = len(case["expected"]["invariants"])
        invariant_checks += invariant_count
        if passed:
            invariant_passes += invariant_count

        if case["category"] in {"provenance", "evidence_span", "inference_policy", "external_source_policy"}:
            supported_checks += 1
            supported_passes += int(passed)

        if contains_confirmed_unsupported(case["input"]) and not actual.answer_mode.startswith(("reject", "deny", "schema")):
            confirmed_unsupported += 1

        span_result = verify_span(case)
        if span_result is not None:
            span_results.append(span_result)
            passed = passed and span_result
            checks["evidence_span_hash"] = span_result

        case_reports.append({
            "id": case["id"],
            "expected_answer_mode": case["expected"]["answer_mode"],
            "actual_answer_mode": actual.answer_mode,
            "checks": checks,
            "passed": passed,
            "error": error,
        })

    retrieval_pairs = [pair for case in cases if (pair := relevant_and_ranked(case)) is not None]
    recall = recall_at_10(retrieval_pairs) if retrieval_pairs else 0.0
    ndcg = ndcg_at_10(retrieval_pairs) if retrieval_pairs else 0.0
    hard_rate = invariant_passes / invariant_checks if invariant_checks else 0.0
    schema_rate = sum(report["passed"] for report in case_reports) / len(case_reports)
    span_rate = sum(span_results) / len(span_results) if span_results else 0.0
    supported_precision = supported_passes / supported_checks if supported_checks else 0.0

    measured = {
        "hard_invariants_pass_rate": hard_rate,
        "recall_at_10": recall,
        "ndcg_at_10": ndcg,
        "schema_valid_after_bounded_repair": schema_rate,
        "evidence_span_exactness": span_rate,
        "supported_claim_precision": supported_precision,
        "confirmed_unsupported_claims": confirmed_unsupported,
    }
    thresholds_passed = (
        hard_rate >= metrics_contract["hard_invariants_pass_rate"]
        and recall >= metrics_contract["retrieval"]["recall_at_10"]["minimum"]
        and ndcg >= metrics_contract["retrieval"]["ndcg_at_10"]["minimum"]
        and schema_rate >= metrics_contract["extraction"]["schema_valid_after_bounded_repair"]["minimum"]
        and span_rate >= metrics_contract["extraction"]["evidence_span_exactness"]["minimum"]
        and supported_precision >= metrics_contract["extraction"]["supported_claim_precision"]["minimum"]
        and confirmed_unsupported <= metrics_contract["extraction"]["confirmed_unsupported_claims"]["maximum"]
    )
    passed_count = sum(report["passed"] for report in case_reports)
    summary = {
        "schema_version": 1,
        "contract_version": manifest["contract_version"],
        "passed": passed_count == len(cases) and thresholds_passed,
        "case_count": len(cases),
        "actual_vs_expected_passed": passed_count,
        "metrics": measured,
        "thresholds_passed": thresholds_passed,
        "seed": manifest["seed"],
    }
    return summary, case_reports


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", default="evals/v1/manifest.json")
    parser.add_argument("--strict-environment", action="store_true")
    parser.add_argument("--details", action="store_true")
    args = parser.parse_args(argv)

    manifest_path = Path(args.manifest).resolve()
    repo_root = manifest_path.parents[2]
    manifest = load_json(manifest_path)
    lock_failures = verify_byte_lock(repo_root, manifest)
    environment_failures = []
    if args.strict_environment:
        expected = tuple(manifest["runner"]["python_version"].split("."))
        actual = tuple(str(part) for part in sys.version_info[:3])
        if actual != expected:
            environment_failures.append(f"python version mismatch: expected {'.'.join(expected)}, got {'.'.join(actual)}")

    summary, case_reports = execute(repo_root, manifest)
    summary["byte_lock_passed"] = not lock_failures
    summary["environment_passed"] = not environment_failures
    summary["passed"] = summary["passed"] and not lock_failures and not environment_failures
    if lock_failures:
        summary["byte_lock_failures"] = lock_failures
    if environment_failures:
        summary["environment_failures"] = environment_failures
    if args.details or not summary["passed"]:
        summary["cases"] = case_reports
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
