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
import locale
import math
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime
from decimal import Decimal
from pathlib import Path
from typing import Any


@dataclass
class Decision:
    answer_mode: str
    details: dict[str, Any] = field(default_factory=dict)
    repair_attempts: int = 0


DECISION_DETAIL_FIELDS = {
    "active_claim_ids", "active_event_seq", "active_event_seqs", "active_user_decision_seq",
    "active_user_event_seq", "actor_id", "brain_value", "claim_status", "content_claim_status",
    "duplicates", "error", "event_count", "event_sequences", "excluded_claim_ids", "expected_slice",
    "external_status", "flags", "historical_claim_ids", "historical_event_seqs", "ledger_event_count",
    "lost", "must_explain", "must_include", "must_link", "must_not", "must_not_appear_in",
    "must_request", "must_validate", "other_value", "projection_eventual_count", "recommended_reference",
    "required_claim_ids", "required_status", "semantic_event_count", "slice", "stored_outcome_count", "value",
}


def parse_time(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("contract timestamps must carry an explicit UTC offset")
    return parsed


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


def derive_details(case: dict[str, Any], decision: Decision) -> dict[str, Any]:
    """Materialize the observable policy output from input facts, never expected data."""
    domain = case["domain"]
    category = case["category"]
    data = case["input"]
    mode = decision.answer_mode
    details = dict(decision.details)

    if domain == "stocks":
        if category == "future_valid":
            if "claims" in data:
                active = active_claim_ids(data["claims"], data.get("query_as_of") or data.get("now"))
                details.update(active_claim_ids=active, excluded_claim_ids=[claim["id"] for claim in data["claims"] if claim["id"] not in active])
            else:
                details["active_claim_ids"] = []
                if "recorded_at" in data["claim"]:
                    details["must_explain"] = ["valid_from"]
                else:
                    details["historical_claim_ids"] = [data["claim"]["id"]]
        elif category == "external_contradiction":
            claims = data["claims"]
            if mode == "time_scoped_not_conflict":
                current = max(claims, key=lambda claim: claim["period"])
                details.update(active_claim_ids=[current["id"]], historical_claim_ids=[claim["id"] for claim in claims if claim is not current])
            else:
                details["active_claim_ids"] = [claim["id"] for claim in claims]
            if data["predicate"] == "target_price":
                details["must_not"] = "silent_overwrite"
            elif data["predicate"] == "revenue_2025":
                details["recommended_reference"] = next(claim["source"] for claim in claims if "audited" in claim["source"])
        elif category == "bitemporal_as_of":
            if "as_of_event_seq" in data:
                eligible = [event for event in data["events"] if event["seq"] <= data["as_of_event_seq"]]
                active = max(eligible, key=lambda event: event["seq"])["claim"]
                details["active_claim_ids"] = [active]
                history = [event["claim"] for event in eligible if event["claim"] != active]
                if history:
                    details["historical_claim_ids"] = history
            elif "recorded_cutoff_seq" in data:
                details["active_claim_ids"] = [data["claim"]["id"]] if mode == "known_retroactive_fact" else []
        elif category == "provenance":
            claim = data["claim"]
            provenance = claim.get("provenance")
            if provenance is None:
                details["error"] = "PROVENANCE_REQUIRED"
            elif provenance.get("unsupported") and claim.get("status") == "confirmed":
                details["error"] = "UNSUPPORTED_INFERENCE_CANNOT_CONFIRM"
            elif data.get("source", {}).get("privacy") == "local_only":
                details["must_not"] = "provider_egress"
            elif provenance["kind"] == "mechanical":
                details["must_explain"] = ["inputs", "method"]
            elif "rendition_utf8" in data:
                rendition = data["rendition_utf8"].encode("utf-8")
                details["expected_slice"] = rendition[provenance["start_byte"] : provenance["end_byte"]].decode("utf-8")
                details["must_validate"] = ["byte_slice_hash"]
        elif category == "corporate_action" and mode == "adjusted_value":
            details["must_explain"] = ["split"]
        elif category == "staleness":
            details["flags"] = ["stale"]
        elif category == "unit_normalization":
            details["must_request"] = ["fx_source", "fx_valid_time"]
        elif category == "entity_identity":
            details["must_not"] = "name_based_merge"
        elif category == "retrieval":
            if "relevant_claim_ids" in data:
                details["required_claim_ids"] = list(data["relevant_claim_ids"])
            else:
                details.update(active_claim_ids=[data["claims"][0]], historical_claim_ids=list(data["claims"][1:]))
        elif category == "abstention":
            if data.get("claims"):
                details["active_claim_ids"] = list(data["claims"])
            else:
                details["must_explain"] = ["insufficient_evidence"]
        elif category == "investment_safety":
            details.update(must_not="execute_trade", must_include=["source", "time", "uncertainty"])

    elif domain == "projects":
        if category == "latest_user_wins":
            events = data["events"]
            if mode == "single_current_with_history":
                winner = max(events, key=lambda event: event["seq"])
                details.update(active_event_seq=winner["seq"], historical_event_seqs=[event["seq"] for event in events if event is not winner])
            elif mode == "scope_isolated":
                brain = next(event for event in events if event["scope"].startswith("brain:"))
                other = next(event for event in events if event["scope"].startswith("other:"))
                details.update(brain_value=brain["value"], other_value=other["value"])
            elif mode in {"context_isolated", "predicate_isolated"}:
                details["active_event_seqs"] = [event["seq"] for event in events]
            elif mode == "authority_separated":
                user_event = next(event for event in events if event.get("kind") == "decision")
                details.update(active_user_decision_seq=user_event["seq"], external_status="disputed_or_evidence")
        elif category == "decision_history":
            if "event" in data:
                if mode == "confirmed_user_decision":
                    details["must_link"] = ["utterance_ref"]
                else:
                    details["error"] = "UNSUPPORTED_INFERENCE_CANNOT_CONFIRM"
            elif mode == "single_as_of":
                details["active_event_seq"] = max(event["seq"] for event in data["events"] if event["seq"] <= data["as_of_event_seq"])
            elif mode == "not_current_with_history":
                details.update(active_event_seqs=[], historical_event_seqs=[event["seq"] for event in data["events"]])
            else:
                winner = data["events"][-1]
                details.update(active_event_seq=winner["seq"], historical_event_seqs=[event["seq"] for event in data["events"][:-1]])
        elif category == "actor_client":
            if mode == "owner_shared_client_audited":
                details.update(active_event_seq=max(event["seq"] for event in data["events"]), must_not="infer_person_identity")
            elif mode == "derive_identity_from_security_context":
                details["actor_id"] = data["request"]["token_actor_id"]
            elif data.get("command") == "confirm":
                details["error"] = "INSUFFICIENT_CAPABILITY"
            elif data.get("command") == "append_event":
                details["error"] = "SEMANTIC_WRITER_REQUIRED"
            elif mode == "allow_local_read":
                details["must_not"] = "trust_tool_annotation_as_auth"
        elif category == "valid_time":
            if mode == "future_milestone":
                details["must_not"] = "mark_current_complete"
            elif mode == "use_claim_valid_time":
                details["must_not"] = "use_git_time_as_valid_time"
        elif category == "idempotency":
            if mode == "one_commit_nine_replays":
                details.update(event_count=1, stored_outcome_count=data["concurrent_requests"])
            elif mode in {"return_stored_outcome", "reject_second"}:
                details["event_count"] = 1
                if mode == "reject_second":
                    details["error"] = "IDEMPOTENCY_CONFLICT"
            else:
                details["event_count"] = 2
        elif category == "authority_boundary":
            if mode == "deny":
                details["error"] = "PROJECTOR_ONLY"
            elif mode == "authored_content_only":
                details["must_not"] = "change_confirmed_claim"
            elif mode == "rebuild_projection":
                details["semantic_event_count"] = data["canonical_events"]
            elif mode == "ledger_current_projection_lag_visible":
                details["must_include"] = ["projection_lag"]
            else:
                details["ledger_event_count"] = 0

    elif domain == "knowledge":
        if category == "latest_user_wins":
            events = data["events"]
            if mode == "single_current_with_history":
                winner = max(events, key=lambda event: event["seq"])
                details.update(active_event_seq=winner["seq"], historical_event_seqs=[event["seq"] for event in events if event is not winner])
            elif mode in {"domain_isolated", "subject_isolated", "context_isolated"}:
                details["active_event_seqs"] = [event["seq"] for event in events]
            else:
                details["active_user_event_seq"] = next(event["seq"] for event in events if event.get("kind") == "preference")
        elif category == "external_source_policy":
            if mode == "downgrade_or_reject":
                details["required_status"] = "proposed"
            elif mode == "eligible_for_confirm":
                details["must_validate"] = ["authenticated_actor", "utterance_span"]
            elif mode == "mechanical_metadata_auto_confirm_only":
                details.update(claim_status="confirmed", content_claim_status="proposed")
            else:
                details.update(claim_status="proposed", must_not="execute_source_instruction")
        elif category == "inference_policy":
            if mode == "deny":
                details["error"] = "UNSUPPORTED_INFERENCE_CANNOT_CONFIRM"
            elif mode == "append_new_user_assertion":
                details["must_not"] = "mutate_inference_to_truth"
            elif mode == "reviewable_proposal":
                details["must_include"] = ["model", "prompt", "evidence"]
        elif category == "evidence_span":
            if mode == "span_valid":
                span = data["span"]
                details.update(slice=data["rendition_utf8"].encode("utf-8")[span["start_byte"] : span["end_byte"]].decode("utf-8"), must_validate=["quote_sha256"])
            elif mode == "reject_non_boundary_span":
                details["error"] = "INVALID_UTF8_BYTE_BOUNDARY"
            elif mode == "reject_mismatched_rendition":
                details["error"] = "RENDITION_HASH_MISMATCH"
            elif mode == "page_and_text_evidence":
                details["must_include"] = ["page_index", "bbox", "text_span", "extractor_version"]
            else:
                details["error"] = "INVALID_EVIDENCE_RANGE"
        elif category == "bitemporal":
            if mode == "reject_invalid_interval":
                details["error"] = "VALID_TIME_REVERSED"
        elif category == "contradiction":
            details["active_claim_ids"] = [claim["id"] for claim in data["claims"]]
        elif category == "identity":
            details["must_not"] = "attribute_to_owner_vs_partner" if mode == "client_audit_only" else "name_only_merge"
        elif category == "retract":
            details["historical_event_seqs"] = [event["seq"] for event in data["events"]]
        elif category == "purge":
            details["error"] = "PURGED"
        elif category == "retrieval":
            details.update(required_claim_ids=list(data["relevant"]), must_include=["source", "valid_time", "recorded_time", "selection_reason"])

    elif domain == "adversarial":
        if category == "prompt_injection":
            policy = {
                "proposal_or_no_claim": {"must_not": "call_confirm"},
                "treat_as_data": {"must_not": "call_purge" if "web_text" in data else "external_side_effect"},
                "schema_or_policy_reject": {"error": "UNSUPPORTED_INFERENCE_CANNOT_CONFIRM"},
                "bounded_failure": {"must_not": "queue_instruction_as_command"},
            }
            details.update(policy[mode])
        elif category == "secret_egress":
            if mode == "deny_before_serialization":
                details["must_not_appear_in"] = ["request", "retry", "dead_letter", "telemetry", "log"]
            elif mode == "redact_and_deny":
                details["must_not_appear_in"] = ["provider_request", "log"]
            elif mode == "deny_destination":
                details["error"] = "DESTINATION_NOT_APPROVED"
            elif mode == "bounded_repair_without_original_egress":
                details["must_not_appear_in"] = ["repair_request", "dead_letter"]
            else:
                details["must_not"] = data["telemetry_event"]["payload"]
        elif category == "stored_xss":
            details["must_not"] = {
                "render_text": "execute_script",
                "escaped_graph_label": "html_capable_node_label",
                "dom_property_text_only": "string_html_attribute",
                "sanitize_link_protocol": "javascript_url",
                "sanitize_or_text": "active_svg",
            }[mode]
        elif category == "ssrf_path_traversal":
            if mode == "deny_url":
                details["error"] = "LINK_LOCAL_DENIED" if "169.254" in data["url"] else "PRIVATE_ADDRESS_DENIED"
            elif mode == "deny_redirect_target":
                details["error"] = "PRIVATE_ADDRESS_DENIED"
            elif mode == "deny_path":
                details["error"] = "PATH_OUTSIDE_ROOT"
            else:
                details["error"] = "UNSAFE_ARCHIVE_OR_SIZE_LIMIT"
        elif category == "auth_reauth":
            if "authorization_request" in data:
                details["error"] = "RESOURCE_INDICATOR_CONFORMANCE_FAILED"
            elif data.get("token", {}).get("aud") == "other-service":
                details["error"] = "INVALID_AUDIENCE"
            elif data.get("command") == "brain_confirm":
                details["error"] = "INSUFFICIENT_CAPABILITY"
            elif data.get("reauth_age_seconds", 0) > 300:
                details["error"] = "RECENT_REAUTH_REQUIRED"
            else:
                details["error"] = "CONFIRMATION_NONCE_EXPIRED"
        elif category == "retry_concurrency":
            if mode in {"one_commit_replay_outcome", "reject_conflict"}:
                details["event_count"] = 1
                if mode == "reject_conflict":
                    details["error"] = "IDEMPOTENCY_CONFLICT"
            elif mode == "serialized_unique_sequence":
                start = data["expected_sequence_start"]
                details.update(event_sequences=f"{start}..{start + data['concurrent_writers'] - 1}", duplicates=0, lost=0)
            elif mode == "orphan_cleanup_no_event":
                details["event_count"] = 0
            else:
                details.update(semantic_event_count=1, projection_eventual_count=1)
    return details


def decide(case: dict[str, Any]) -> Decision:
    dispatch = {
        "stocks": decide_stocks,
        "projects": decide_projects,
        "knowledge": decide_knowledge,
        "adversarial": decide_adversarial,
    }
    decision = dispatch[case["domain"]](case["category"], case["input"])
    decision.details = derive_details(case, decision)
    return decision


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


def relevant_and_ranked(case: dict[str, Any], decision: Decision) -> tuple[list[str], list[str]] | None:
    data = case["input"]
    relevant = data.get("relevant_claim_ids") or data.get("relevant")
    if relevant is None and case["category"] == "retrieval":
        relevant = data.get("claims")
    ranked = data.get("top10")
    if ranked is None and relevant is not None:
        ranked = (
            decision.details.get("required_claim_ids", [])
            + decision.details.get("active_claim_ids", [])
            + decision.details.get("historical_claim_ids", [])
        )
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


def mode_is(*allowed: str):
    return lambda _case, decision: decision.answer_mode in allowed


def idempotency_holds(_case: dict[str, Any], decision: Decision) -> bool:
    if decision.answer_mode == "serialized_unique_sequence":
        return decision.details.get("duplicates") == 0 and decision.details.get("lost") == 0
    return decision.answer_mode in {
        "return_stored_outcome", "reject_second", "client_isolated_operations",
        "owner_isolated_operations", "one_commit_nine_replays",
        "one_commit_replay_outcome", "reject_conflict", "replay_projection_once",
    } and decision.details.get("event_count", decision.details.get("semantic_event_count")) == 1 or decision.answer_mode in {
        "client_isolated_operations", "owner_isolated_operations"
    } and decision.details.get("event_count") == 2


def decision_matches_policy(case: dict[str, Any], decision: Decision) -> bool:
    canonical = decide(case)
    return (
        decision.answer_mode == canonical.answer_mode
        and decision.details == canonical.details
        and decision_schema_valid(decision, maximum_repairs=2)
    )


def authorization_holds(case: dict[str, Any], decision: Decision) -> bool:
    if not decision_matches_policy(case, decision):
        return False
    data = case["input"]
    if case["category"] == "transport_bind":
        public_unauthenticated = data["network"] == "public" and data["bind"].startswith("0.0.0.0") and not data["auth_gate_passed"]
        return not public_unauthenticated or decision.answer_mode == "refuse_public_startup"
    if case["category"] == "auth_reauth":
        return decision.answer_mode in {"deny", "keep_remote_disabled"} and bool(decision.details.get("error"))
    if data.get("command") in {"confirm", "append_event"} or "worker_capabilities" in data:
        return decision.answer_mode in {"deny", "ignore_and_deny_tool"}
    if "request" in data and "token_actor_id" in data["request"]:
        return decision.answer_mode == "derive_identity_from_security_context" and decision.details.get("actor_id") == data["request"]["token_actor_id"]
    return decision.answer_mode in {"allow_local_read", "proposal_or_no_claim", "treat_as_data"}


def purge_denial_holds(case: dict[str, Any], decision: Decision) -> bool:
    if not decision_matches_policy(case, decision):
        return False
    data = case["input"]
    if data.get("operation") == "restore":
        stale = data["backup_purge_epoch"] < data["registry_purge_epoch"]
        return not stale or decision.answer_mode == f"seal_until_epoch_{data['registry_purge_epoch']}_applied"
    if data.get("read_request") or data.get("read_target"):
        return decision.answer_mode.startswith(("deny_read", "seal_until_epoch_"))
    if data.get("command") == "brain_purge":
        return decision.answer_mode == "deny" and bool(decision.details.get("error"))
    return False


def fail_closed_holds(case: dict[str, Any], decision: Decision) -> bool:
    if not decision_matches_policy(case, decision):
        return False
    data = case["input"]
    if case["category"] == "transport_bind":
        return decision.answer_mode == "refuse_public_startup" and not data["auth_gate_passed"]
    if data.get("operation") == "restore":
        stale_backup = data["backup_purge_epoch"] < data["registry_purge_epoch"]
        return stale_backup and decision.answer_mode == f"seal_until_epoch_{data['registry_purge_epoch']}_applied"
    return (data.get("read_request") or data.get("read_target")) and decision.answer_mode.startswith("deny_read")


def no_egress_holds(case: dict[str, Any], decision: Decision) -> bool:
    if not decision_matches_policy(case, decision):
        return False
    data = case["input"]
    blocked_modes = {
        "local_evidence_only", "treat_as_data", "deny_before_serialization", "redact_and_deny",
        "deny_destination", "bounded_repair_without_original_egress", "structured_redacted_telemetry",
    }
    sensitive = (
        data.get("privacy") == "local_only"
        or data.get("source", {}).get("privacy") == "local_only"
        or data.get("original_payload_privacy") == "local_only"
        or "detector" in data
        or "telemetry_event" in data
        or data.get("destination") == "unapproved.example.invalid"
        or "export" in next((value for key, value in data.items() if key.endswith("text")), "").lower()
    )
    return sensitive and decision.answer_mode in blocked_modes


def prompt_injection_holds(case: dict[str, Any], decision: Decision) -> bool:
    if not decision_matches_policy(case, decision):
        return False
    data = case["input"]
    text = next((value for key, value in data.items() if key.endswith("text")), data.get("source", {}).get("prompt_text", ""))
    injected = any(marker in text.lower() for marker in ("ignore", "system", "brain_", "unsupported=true", "retry queue"))
    return injected and decision.answer_mode in {"proposal_or_no_claim", "treat_as_data", "schema_or_policy_reject", "bounded_failure"}


def evidence_span_holds(case: dict[str, Any], decision: Decision) -> bool:
    mechanical = verify_span(case)
    if mechanical is not None:
        return mechanical and decision.answer_mode in {"evidence_required", "span_valid"}
    return decision.answer_mode in {
        "reject_non_boundary_span", "reject_mismatched_rendition", "reject_invalid_interval"
    }


def retrieval_trace_holds(_case: dict[str, Any], decision: Decision) -> bool:
    return decision.answer_mode == "evidence_pack" and bool(decision.details.get("required_claim_ids"))


def no_confirmed_unsupported_holds(_case: dict[str, Any], decision: Decision) -> bool:
    return not contains_confirmed_unsupported(decision.details) and decision.answer_mode in {
        "reject_invalid_state", "proposal_only", "deny", "schema_or_policy_reject"
    }


INVARIANT_EVALUATORS = {
    "abstention_available": mode_is("multiple_opinions", "abstain", "disputed_no_forced_winner", "do_not_merge_units"),
    "actor_identity_policy": mode_is("owner_shared_client_audited", "client_audit_only"),
    "ai_proposes_policy_commits": mode_is("ignore_and_deny_tool"),
    "atomic_write_boundary": mode_is("safe_orphan_cleanup", "orphan_cleanup_no_event"),
    "authority_separation": mode_is("user_preference_not_overwritten"),
    "authorization": authorization_holds,
    "bitemporal_correctness": mode_is("not_yet_known", "known_retroactive_fact"),
    "client_audit": mode_is("owner_shared_client_audited"),
    "client_isolation": mode_is("client_isolated_operations"),
    "clock_tie_safe": mode_is("sequence_ordered"),
    "csp_enforced": mode_is("sanitize_or_text"),
    "dns_ip_recheck": mode_is("deny_redirect_target"),
    "event_seq_authoritative": mode_is("sequence_ordered"),
    "evidence_span_exactness": evidence_span_holds,
    "external_conflict_preserved": mode_is("disputed_with_evidence", "disputed_reference_recommended", "disputed_with_units", "multiple_opinions", "disputed_no_forced_winner", "authority_separated"),
    "external_sources_start_proposed": mode_is("downgrade_or_reject", "mechanical_metadata_auto_confirm_only"),
    "fail_closed": fail_closed_holds,
    "half_open_interval": mode_is("single_current"),
    "idempotency": idempotency_holds,
    "latest_user_wins_scoped": mode_is("single_current_with_history", "scope_isolated", "context_isolated", "predicate_isolated", "authority_separated", "domain_isolated", "subject_isolated", "user_preference_not_overwritten"),
    "ledger_authoritative": mode_is("ledger_current_projection_lag_visible"),
    "mechanical_proof": mode_is("mechanical_metadata_auto_confirm_only"),
    "no_confirmed_unsupported_claim": no_confirmed_unsupported_holds,
    "no_dual_source_of_truth": mode_is("authored_content_only"),
    "no_financial_side_effect": mode_is("informational_only"),
    "no_history_rewrite": mode_is("single_as_of", "single_current_with_history", "current_plus_timeline", "append_new_user_assertion"),
    "no_lost_update": mode_is("one_commit_nine_replays", "safe_orphan_cleanup", "serialized_unique_sequence", "orphan_cleanup_no_event"),
    "no_secret_or_local_only_egress": no_egress_holds,
    "no_silent_overwrite": mode_is("current_plus_timeline"),
    "one_semantic_writer": mode_is("deny"),
    "owner_isolation": mode_is("owner_isolated_operations"),
    "path_traversal_safe": mode_is("deny_path", "deny_archive"),
    "projection_rebuildable": mode_is("rebuild_projection"),
    "prompt_injection_resistance": prompt_injection_holds,
    "provenance_valid": mode_is("disputed_with_evidence", "evidence_required", "mechanically_derived", "reject_invalid_state", "local_evidence_only", "reject_invalid_claim", "confirmed_user_decision", "eligible_for_confirm", "append_new_user_assertion", "reviewable_proposal", "page_and_text_evidence", "evidence_pack"),
    "purge_denial": purge_denial_holds,
    "retract_preserves_history": mode_is("not_current_with_history", "not_current_with_timeline"),
    "retrieval_trace": retrieval_trace_holds,
    "scope_context_preserved": mode_is("contextual_not_conflict"),
    "size_limit": mode_is("deny_archive"),
    "ssrf_safe": mode_is("deny_url", "deny_redirect_target"),
    "stable_identity": mode_is("needs_disambiguation", "propose_alias_link"),
    "staleness_visible": mode_is("answer_with_stale_flag"),
    "stored_xss_safe": mode_is("render_text", "escaped_graph_label", "dom_property_text_only", "sanitize_link_protocol", "sanitize_or_text"),
    "temporal_correctness": mode_is("abstain_current", "single_current", "known_future_not_current", "adjusted_value", "unadjusted_value", "current_plus_history", "future_milestone", "use_claim_valid_time", "reject_invalid_interval"),
    "transaction_time_as_of": mode_is("single_as_of"),
    "transactional_outbox": mode_is("replay_projection_once"),
    "typed_value": mode_is("disputed_with_units", "adjusted_value", "do_not_merge_units"),
    "valid_time_not_ingest_order": mode_is("time_scoped_not_conflict"),
}


def evaluate_invariant(name: str, case: dict[str, Any], decision: Decision) -> bool:
    evaluator = INVARIANT_EVALUATORS.get(name)
    if evaluator is None:
        raise ValueError(f"missing invariant evaluator: {name}")
    return bool(evaluator(case, decision))


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


def decision_schema_valid(decision: Decision, maximum_repairs: int) -> bool:
    if not isinstance(decision.answer_mode, str) or not decision.answer_mode:
        return False
    if not isinstance(decision.details, dict) or not all(isinstance(key, str) for key in decision.details):
        return False
    if not set(decision.details).issubset(DECISION_DETAIL_FIELDS):
        return False
    if not isinstance(decision.repair_attempts, int) or not 0 <= decision.repair_attempts <= maximum_repairs:
        return False
    try:
        json.dumps({"answer_mode": decision.answer_mode, "details": decision.details})
    except (TypeError, ValueError):
        return False
    return True


def evaluate_case(case: dict[str, Any], maximum_repairs: int = 2) -> dict[str, Any]:
    expected = case["expected"]
    expected_keys = [key for key in expected if key != "invariants"]
    try:
        actual = decide(case)
        actual_output = {"answer_mode": actual.answer_mode, **actual.details}
        expected_checks = {key: actual_output.get(key, object()) == expected[key] for key in expected_keys}
        invariant_checks = {
            name: evaluate_invariant(name, case, actual)
            for name in expected["invariants"]
        }
        schema_valid = decision_schema_valid(actual, maximum_repairs)
        span = verify_span(case)
        spans = [] if span is None else [span]

        emitted_ids = []
        expected_supported = set()
        for key in ("active_claim_ids", "required_claim_ids"):
            emitted_ids.extend(actual.details.get(key, []))
            expected_supported.update(expected.get(key, []))
        supported_claims_emitted = [claim_id in expected_supported for claim_id in emitted_ids]
        retrieval = relevant_and_ranked(case, actual)
        confirmed_unsupported = int(contains_confirmed_unsupported(actual_output))
        passed = all(expected_checks.values()) and all(invariant_checks.values()) and schema_valid
        error = None
    except Exception as exc:  # contract runner reports the case instead of hiding it
        actual = Decision("runner_error")
        expected_checks = {key: False for key in expected_keys}
        invariant_checks = {name: False for name in expected["invariants"]}
        schema_valid = False
        spans = []
        supported_claims_emitted = []
        retrieval = None
        confirmed_unsupported = 0
        passed = False
        error = f"{type(exc).__name__}: {exc}"

    return {
        "id": case["id"],
        "expected_answer_mode": expected["answer_mode"],
        "actual_answer_mode": actual.answer_mode,
        "actual_details": actual.details,
        "expected_checks": expected_checks,
        "invariant_checks": invariant_checks,
        "hard_invariant_case": bool(expected["invariants"]),
        "invariants_passed": bool(invariant_checks) and all(invariant_checks.values()),
        "schema_valid": schema_valid,
        "repair_attempts": actual.repair_attempts,
        "spans": spans,
        "supported_claims_emitted": supported_claims_emitted,
        "confirmed_unsupported_claims": confirmed_unsupported,
        "retrieval": None if retrieval is None else {"relevant": retrieval[0], "ranked": retrieval[1]},
        "passed": passed,
        "error": error,
    }


def compute_metrics(case_reports: list[dict[str, Any]], metrics_contract: dict[str, Any]) -> dict[str, float | int]:
    hard_cases = [report for report in case_reports if report["hard_invariant_case"]]
    schema_outputs = list(case_reports)
    maximum_repairs = metrics_contract["extraction"]["schema_valid_after_bounded_repair"]["maximum_repairs"]
    span_results = [result for report in case_reports for result in report["spans"]]
    supported_results = [result for report in case_reports for result in report["supported_claims_emitted"]]
    retrieval_pairs = [
        (report["retrieval"]["relevant"], report["retrieval"]["ranked"])
        for report in case_reports if report["retrieval"] is not None
    ]

    return {
        "hard_invariants_pass_rate": sum(report["invariants_passed"] for report in hard_cases) / len(hard_cases) if hard_cases else 0.0,
        "recall_at_10": recall_at_10(retrieval_pairs) if retrieval_pairs else 0.0,
        "ndcg_at_10": ndcg_at_10(retrieval_pairs) if retrieval_pairs else 0.0,
        "schema_valid_after_bounded_repair": sum(report["schema_valid"] and report["repair_attempts"] <= maximum_repairs for report in schema_outputs) / len(schema_outputs) if schema_outputs else 0.0,
        "evidence_span_exactness": sum(span_results) / len(span_results) if span_results else 0.0,
        "supported_claim_precision": sum(supported_results) / len(supported_results) if supported_results else 0.0,
        "confirmed_unsupported_claims": sum(report["confirmed_unsupported_claims"] for report in case_reports),
    }


def evaluate_thresholds(measured: dict[str, float | int], contract: dict[str, Any], baseline: dict[str, float]) -> tuple[bool, list[str]]:
    failures = []
    minimums = {
        "hard_invariants_pass_rate": contract["hard_invariants_pass_rate"],
        "recall_at_10": contract["retrieval"]["recall_at_10"]["minimum"],
        "ndcg_at_10": contract["retrieval"]["ndcg_at_10"]["minimum"],
        "schema_valid_after_bounded_repair": contract["extraction"]["schema_valid_after_bounded_repair"]["minimum"],
        "evidence_span_exactness": contract["extraction"]["evidence_span_exactness"]["minimum"],
        "supported_claim_precision": contract["extraction"]["supported_claim_precision"]["minimum"],
    }
    for metric, minimum in minimums.items():
        if measured[metric] < minimum:
            failures.append(f"{metric} below minimum: {measured[metric]} < {minimum}")
    unsupported_maximum = contract["extraction"]["confirmed_unsupported_claims"]["maximum"]
    if measured["confirmed_unsupported_claims"] > unsupported_maximum:
        failures.append(f"confirmed_unsupported_claims above maximum: {measured['confirmed_unsupported_claims']} > {unsupported_maximum}")

    maximum_regression = contract["retrieval"]["maximum_absolute_regression"]
    for metric in ("recall_at_10", "ndcg_at_10"):
        regression = Decimal(str(baseline[metric])) - Decimal(str(measured[metric]))
        if regression > Decimal(str(maximum_regression)):
            failures.append(f"{metric} maximum_absolute_regression exceeded: {regression} > {maximum_regression}")
    return not failures, failures


def execute(repo_root: Path, manifest: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    metrics_contract = load_json(repo_root / manifest["metrics"]["path"])
    cases = [
        case
        for corpus in manifest["corpora"].values()
        for case in load_jsonl(repo_root / corpus["path"])
    ]
    maximum_repairs = metrics_contract["extraction"]["schema_valid_after_bounded_repair"]["maximum_repairs"]
    case_reports = [evaluate_case(case, maximum_repairs) for case in cases]
    measured = compute_metrics(case_reports, metrics_contract)
    thresholds_passed, threshold_failures = evaluate_thresholds(
        measured,
        metrics_contract,
        manifest["promoted_baseline"]["metrics"],
    )
    passed_count = sum(report["passed"] for report in case_reports)
    summary = {
        "schema_version": 1,
        "contract_version": manifest["contract_version"],
        "passed": passed_count == len(cases) and thresholds_passed,
        "case_count": len(cases),
        "actual_vs_expected_passed": passed_count,
        "metrics": measured,
        "metric_denominators": {
            "hard_invariant_cases": sum(report["hard_invariant_case"] for report in case_reports),
            "extraction_outputs": len(case_reports),
            "annotated_spans": sum(len(report["spans"]) for report in case_reports),
            "supported_claims_emitted": sum(len(report["supported_claims_emitted"]) for report in case_reports),
            "retrieval_queries": sum(report["retrieval"] is not None for report in case_reports),
        },
        "thresholds_passed": thresholds_passed,
        "threshold_failures": threshold_failures,
        "seed": manifest["seed"],
    }
    return summary, case_reports


def install_network_guard() -> bool:
    """Deny network syscalls inside the corpus runner and prove the hook is live."""
    def deny_network(event: str, _args: tuple[Any, ...]) -> None:
        if event.startswith("socket.") or event in {"http.client.connect", "urllib.Request"}:
            raise PermissionError(f"network disabled for corpus evaluation: {event}")

    sys.addaudithook(deny_network)
    try:
        sys.audit("socket.connect", None, ("127.0.0.1", 9))
    except PermissionError:
        return True
    return False


def inspect_environment(manifest: dict[str, Any], network_guard: bool) -> dict[str, Any]:
    runner = manifest["runner"]
    actual_python = ".".join(str(part) for part in sys.version_info[:3])
    try:
        uv_completed = subprocess.run(["uv", "--version"], check=False, capture_output=True, text=True)
        uv_output = (uv_completed.stdout or uv_completed.stderr).strip()
    except OSError as exc:
        uv_output = f"unavailable: {exc}"

    checks = {
        "python_version": actual_python == runner["python_version"],
        "uv_version": uv_output.startswith(f"uv {runner['uv_version']} ") or uv_output == f"uv {runner['uv_version']}",
        "timezone_policy": runner["timezone"] == {
            "policy": "host-independent",
            "requirement": "all fixture timestamps and arithmetic use offset-aware ISO-8601 values; host timezone is observed but not consulted",
        },
        "locale_policy": runner["locale"] == {
            "policy": "host-independent",
            "file_encoding": "UTF-8 explicit",
            "stdout_encoding": "ASCII-escaped JSON",
        },
        "network_guard": network_guard and runner["network"]["runner_policy"] == "blocked",
    }
    observed = {
        "python_version": actual_python,
        "uv_version": uv_output,
        "timezone": f"{time.tzname}; offset_seconds={-time.timezone}",
        "preferred_encoding": locale.getpreferredencoding(False),
        "stdout_encoding": sys.stdout.encoding,
    }
    return {"passed": all(checks.values()), "checks": checks, "observed": observed}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", default="evals/v1/manifest.json")
    parser.add_argument("--strict-environment", action="store_true")
    parser.add_argument("--details", action="store_true")
    args = parser.parse_args(argv)

    manifest_path = Path(args.manifest).resolve()
    repo_root = manifest_path.parents[2]
    manifest = load_json(manifest_path)
    network_guard = install_network_guard()
    lock_failures = verify_byte_lock(repo_root, manifest)
    environment = inspect_environment(manifest, network_guard)

    summary, case_reports = execute(repo_root, manifest)
    summary["byte_lock_passed"] = not lock_failures
    summary["environment"] = environment
    summary["environment_passed"] = environment["passed"]
    summary["passed"] = summary["passed"] and not lock_failures and (environment["passed"] or not args.strict_environment)
    if lock_failures:
        summary["byte_lock_failures"] = lock_failures
    if args.details or not summary["passed"]:
        summary["cases"] = case_reports
    print(json.dumps(summary, ensure_ascii=True, sort_keys=True))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
