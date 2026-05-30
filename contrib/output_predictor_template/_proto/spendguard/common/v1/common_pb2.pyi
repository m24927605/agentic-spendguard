from google.protobuf import timestamp_pb2 as _timestamp_pb2
from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Iterable as _Iterable, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class TraceContext(_message.Message):
    __slots__ = ("trace_id", "span_id", "parent_span_id", "trace_state")
    TRACE_ID_FIELD_NUMBER: _ClassVar[int]
    SPAN_ID_FIELD_NUMBER: _ClassVar[int]
    PARENT_SPAN_ID_FIELD_NUMBER: _ClassVar[int]
    TRACE_STATE_FIELD_NUMBER: _ClassVar[int]
    trace_id: str
    span_id: str
    parent_span_id: str
    trace_state: str
    def __init__(self, trace_id: _Optional[str] = ..., span_id: _Optional[str] = ..., parent_span_id: _Optional[str] = ..., trace_state: _Optional[str] = ...) -> None: ...

class SpendGuardIds(_message.Message):
    __slots__ = ("run_id", "step_id", "llm_call_id", "tool_call_id", "decision_id", "snapshot_id")
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    STEP_ID_FIELD_NUMBER: _ClassVar[int]
    LLM_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    TOOL_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    DECISION_ID_FIELD_NUMBER: _ClassVar[int]
    SNAPSHOT_ID_FIELD_NUMBER: _ClassVar[int]
    run_id: str
    step_id: str
    llm_call_id: str
    tool_call_id: str
    decision_id: str
    snapshot_id: str
    def __init__(self, run_id: _Optional[str] = ..., step_id: _Optional[str] = ..., llm_call_id: _Optional[str] = ..., tool_call_id: _Optional[str] = ..., decision_id: _Optional[str] = ..., snapshot_id: _Optional[str] = ...) -> None: ...

class UnitRef(_message.Message):
    __slots__ = ("unit_id", "kind", "currency", "unit_name", "token_kind", "model_family", "credit_program")
    class Kind(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        KIND_UNSPECIFIED: _ClassVar[UnitRef.Kind]
        MONETARY: _ClassVar[UnitRef.Kind]
        TOKEN: _ClassVar[UnitRef.Kind]
        CREDIT: _ClassVar[UnitRef.Kind]
        NON_MONETARY: _ClassVar[UnitRef.Kind]
    KIND_UNSPECIFIED: UnitRef.Kind
    MONETARY: UnitRef.Kind
    TOKEN: UnitRef.Kind
    CREDIT: UnitRef.Kind
    NON_MONETARY: UnitRef.Kind
    UNIT_ID_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    CURRENCY_FIELD_NUMBER: _ClassVar[int]
    UNIT_NAME_FIELD_NUMBER: _ClassVar[int]
    TOKEN_KIND_FIELD_NUMBER: _ClassVar[int]
    MODEL_FAMILY_FIELD_NUMBER: _ClassVar[int]
    CREDIT_PROGRAM_FIELD_NUMBER: _ClassVar[int]
    unit_id: str
    kind: UnitRef.Kind
    currency: str
    unit_name: str
    token_kind: str
    model_family: str
    credit_program: str
    def __init__(self, unit_id: _Optional[str] = ..., kind: _Optional[_Union[UnitRef.Kind, str]] = ..., currency: _Optional[str] = ..., unit_name: _Optional[str] = ..., token_kind: _Optional[str] = ..., model_family: _Optional[str] = ..., credit_program: _Optional[str] = ...) -> None: ...

class Amount(_message.Message):
    __slots__ = ("atomic", "unit")
    ATOMIC_FIELD_NUMBER: _ClassVar[int]
    UNIT_FIELD_NUMBER: _ClassVar[int]
    atomic: str
    unit: UnitRef
    def __init__(self, atomic: _Optional[str] = ..., unit: _Optional[_Union[UnitRef, _Mapping]] = ...) -> None: ...

class PricingFreeze(_message.Message):
    __slots__ = ("pricing_version", "price_snapshot_hash", "fx_rate_version", "unit_conversion_version")
    PRICING_VERSION_FIELD_NUMBER: _ClassVar[int]
    PRICE_SNAPSHOT_HASH_FIELD_NUMBER: _ClassVar[int]
    FX_RATE_VERSION_FIELD_NUMBER: _ClassVar[int]
    UNIT_CONVERSION_VERSION_FIELD_NUMBER: _ClassVar[int]
    pricing_version: str
    price_snapshot_hash: bytes
    fx_rate_version: str
    unit_conversion_version: str
    def __init__(self, pricing_version: _Optional[str] = ..., price_snapshot_hash: _Optional[bytes] = ..., fx_rate_version: _Optional[str] = ..., unit_conversion_version: _Optional[str] = ...) -> None: ...

class Fencing(_message.Message):
    __slots__ = ("epoch", "scope_id", "workload_instance_id")
    EPOCH_FIELD_NUMBER: _ClassVar[int]
    SCOPE_ID_FIELD_NUMBER: _ClassVar[int]
    WORKLOAD_INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    epoch: int
    scope_id: str
    workload_instance_id: str
    def __init__(self, epoch: _Optional[int] = ..., scope_id: _Optional[str] = ..., workload_instance_id: _Optional[str] = ...) -> None: ...

class Replay(_message.Message):
    __slots__ = ("ledger_transaction_id", "operation_kind", "audit_decision_event_id", "recorded_at", "operation_id", "status_code", "decision_id", "projection_ids", "ttl_expires_at")
    class StatusCode(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        STATUS_CODE_UNSPECIFIED: _ClassVar[Replay.StatusCode]
        POSTED: _ClassVar[Replay.StatusCode]
        VOIDED: _ClassVar[Replay.StatusCode]
        PENDING: _ClassVar[Replay.StatusCode]
    STATUS_CODE_UNSPECIFIED: Replay.StatusCode
    POSTED: Replay.StatusCode
    VOIDED: Replay.StatusCode
    PENDING: Replay.StatusCode
    LEDGER_TRANSACTION_ID_FIELD_NUMBER: _ClassVar[int]
    OPERATION_KIND_FIELD_NUMBER: _ClassVar[int]
    AUDIT_DECISION_EVENT_ID_FIELD_NUMBER: _ClassVar[int]
    RECORDED_AT_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    STATUS_CODE_FIELD_NUMBER: _ClassVar[int]
    DECISION_ID_FIELD_NUMBER: _ClassVar[int]
    PROJECTION_IDS_FIELD_NUMBER: _ClassVar[int]
    TTL_EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    ledger_transaction_id: str
    operation_kind: str
    audit_decision_event_id: str
    recorded_at: _timestamp_pb2.Timestamp
    operation_id: str
    status_code: Replay.StatusCode
    decision_id: str
    projection_ids: _containers.RepeatedScalarFieldContainer[str]
    ttl_expires_at: _timestamp_pb2.Timestamp
    def __init__(self, ledger_transaction_id: _Optional[str] = ..., operation_kind: _Optional[str] = ..., audit_decision_event_id: _Optional[str] = ..., recorded_at: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ..., operation_id: _Optional[str] = ..., status_code: _Optional[_Union[Replay.StatusCode, str]] = ..., decision_id: _Optional[str] = ..., projection_ids: _Optional[_Iterable[str]] = ..., ttl_expires_at: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class FullResponsePayload(_message.Message):
    __slots__ = ("payload", "expires_at")
    PAYLOAD_FIELD_NUMBER: _ClassVar[int]
    EXPIRES_AT_FIELD_NUMBER: _ClassVar[int]
    payload: bytes
    expires_at: _timestamp_pb2.Timestamp
    def __init__(self, payload: _Optional[bytes] = ..., expires_at: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...

class Error(_message.Message):
    __slots__ = ("code", "message", "details")
    class Code(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        CODE_UNSPECIFIED: _ClassVar[Error.Code]
        FENCING_EPOCH_STALE: _ClassVar[Error.Code]
        LOCK_ORDER_TOKEN_MISMATCH: _ClassVar[Error.Code]
        PRICING_VERSION_UNKNOWN: _ClassVar[Error.Code]
        UNIT_NORMALIZATION_REQUIRED: _ClassVar[Error.Code]
        BUDGET_EXHAUSTED: _ClassVar[Error.Code]
        DEADLOCK_TIMEOUT: _ClassVar[Error.Code]
        SYNC_REPLICA_UNAVAILABLE: _ClassVar[Error.Code]
        TENANT_DISABLED: _ClassVar[Error.Code]
        SCHEMA_BUNDLE_UNKNOWN: _ClassVar[Error.Code]
        SIGNATURE_INVALID: _ClassVar[Error.Code]
        AUDIT_INVARIANT_VIOLATED: _ClassVar[Error.Code]
        DUPLICATE_DECISION_EVENT: _ClassVar[Error.Code]
        RESERVATION_STATE_CONFLICT: _ClassVar[Error.Code]
        PRICING_FREEZE_MISMATCH: _ClassVar[Error.Code]
        OVERRUN_RESERVATION: _ClassVar[Error.Code]
        RESERVATION_TTL_EXPIRED: _ClassVar[Error.Code]
        MULTI_RESERVATION_COMMIT_DEFERRED: _ClassVar[Error.Code]
    CODE_UNSPECIFIED: Error.Code
    FENCING_EPOCH_STALE: Error.Code
    LOCK_ORDER_TOKEN_MISMATCH: Error.Code
    PRICING_VERSION_UNKNOWN: Error.Code
    UNIT_NORMALIZATION_REQUIRED: Error.Code
    BUDGET_EXHAUSTED: Error.Code
    DEADLOCK_TIMEOUT: Error.Code
    SYNC_REPLICA_UNAVAILABLE: Error.Code
    TENANT_DISABLED: Error.Code
    SCHEMA_BUNDLE_UNKNOWN: Error.Code
    SIGNATURE_INVALID: Error.Code
    AUDIT_INVARIANT_VIOLATED: Error.Code
    DUPLICATE_DECISION_EVENT: Error.Code
    RESERVATION_STATE_CONFLICT: Error.Code
    PRICING_FREEZE_MISMATCH: Error.Code
    OVERRUN_RESERVATION: Error.Code
    RESERVATION_TTL_EXPIRED: Error.Code
    MULTI_RESERVATION_COMMIT_DEFERRED: Error.Code
    class DetailsEntry(_message.Message):
        __slots__ = ("key", "value")
        KEY_FIELD_NUMBER: _ClassVar[int]
        VALUE_FIELD_NUMBER: _ClassVar[int]
        key: str
        value: str
        def __init__(self, key: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...
    CODE_FIELD_NUMBER: _ClassVar[int]
    MESSAGE_FIELD_NUMBER: _ClassVar[int]
    DETAILS_FIELD_NUMBER: _ClassVar[int]
    code: Error.Code
    message: str
    details: _containers.ScalarMap[str, str]
    def __init__(self, code: _Optional[_Union[Error.Code, str]] = ..., message: _Optional[str] = ..., details: _Optional[_Mapping[str, str]] = ...) -> None: ...

class BudgetClaim(_message.Message):
    __slots__ = ("budget_id", "unit", "amount_atomic", "direction", "window_instance_id")
    class Direction(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        DIRECTION_UNSPECIFIED: _ClassVar[BudgetClaim.Direction]
        DEBIT: _ClassVar[BudgetClaim.Direction]
        CREDIT: _ClassVar[BudgetClaim.Direction]
    DIRECTION_UNSPECIFIED: BudgetClaim.Direction
    DEBIT: BudgetClaim.Direction
    CREDIT: BudgetClaim.Direction
    BUDGET_ID_FIELD_NUMBER: _ClassVar[int]
    UNIT_FIELD_NUMBER: _ClassVar[int]
    AMOUNT_ATOMIC_FIELD_NUMBER: _ClassVar[int]
    DIRECTION_FIELD_NUMBER: _ClassVar[int]
    WINDOW_INSTANCE_ID_FIELD_NUMBER: _ClassVar[int]
    budget_id: str
    unit: UnitRef
    amount_atomic: str
    direction: BudgetClaim.Direction
    window_instance_id: str
    def __init__(self, budget_id: _Optional[str] = ..., unit: _Optional[_Union[UnitRef, _Mapping]] = ..., amount_atomic: _Optional[str] = ..., direction: _Optional[_Union[BudgetClaim.Direction, str]] = ..., window_instance_id: _Optional[str] = ...) -> None: ...

class LockOrderToken(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: str
    def __init__(self, value: _Optional[str] = ...) -> None: ...

class CloudEvent(_message.Message):
    __slots__ = ("specversion", "type", "source", "id", "time", "datacontenttype", "data", "tenant_id", "run_id", "decision_id", "schema_bundle_id", "producer_id", "producer_sequence", "producer_signature", "signing_key_id", "predicted_a_tokens", "predicted_b_tokens", "predicted_c_tokens", "reserved_strategy", "prediction_strategy_used", "prediction_policy_used", "tokenizer_tier", "tokenizer_version_id", "prediction_confidence", "prediction_sample_size", "cold_start_layer_used", "run_projection_at_decision_atomic", "run_predicted_remaining_steps", "run_steps_completed_so_far", "actual_input_tokens", "actual_output_tokens", "delta_b_ratio", "delta_c_ratio")
    SPECVERSION_FIELD_NUMBER: _ClassVar[int]
    TYPE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    ID_FIELD_NUMBER: _ClassVar[int]
    TIME_FIELD_NUMBER: _ClassVar[int]
    DATACONTENTTYPE_FIELD_NUMBER: _ClassVar[int]
    DATA_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    RUN_ID_FIELD_NUMBER: _ClassVar[int]
    DECISION_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_BUNDLE_ID_FIELD_NUMBER: _ClassVar[int]
    PRODUCER_ID_FIELD_NUMBER: _ClassVar[int]
    PRODUCER_SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    PRODUCER_SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    SIGNING_KEY_ID_FIELD_NUMBER: _ClassVar[int]
    PREDICTED_A_TOKENS_FIELD_NUMBER: _ClassVar[int]
    PREDICTED_B_TOKENS_FIELD_NUMBER: _ClassVar[int]
    PREDICTED_C_TOKENS_FIELD_NUMBER: _ClassVar[int]
    RESERVED_STRATEGY_FIELD_NUMBER: _ClassVar[int]
    PREDICTION_STRATEGY_USED_FIELD_NUMBER: _ClassVar[int]
    PREDICTION_POLICY_USED_FIELD_NUMBER: _ClassVar[int]
    TOKENIZER_TIER_FIELD_NUMBER: _ClassVar[int]
    TOKENIZER_VERSION_ID_FIELD_NUMBER: _ClassVar[int]
    PREDICTION_CONFIDENCE_FIELD_NUMBER: _ClassVar[int]
    PREDICTION_SAMPLE_SIZE_FIELD_NUMBER: _ClassVar[int]
    COLD_START_LAYER_USED_FIELD_NUMBER: _ClassVar[int]
    RUN_PROJECTION_AT_DECISION_ATOMIC_FIELD_NUMBER: _ClassVar[int]
    RUN_PREDICTED_REMAINING_STEPS_FIELD_NUMBER: _ClassVar[int]
    RUN_STEPS_COMPLETED_SO_FAR_FIELD_NUMBER: _ClassVar[int]
    ACTUAL_INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    ACTUAL_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    DELTA_B_RATIO_FIELD_NUMBER: _ClassVar[int]
    DELTA_C_RATIO_FIELD_NUMBER: _ClassVar[int]
    specversion: str
    type: str
    source: str
    id: str
    time: _timestamp_pb2.Timestamp
    datacontenttype: str
    data: bytes
    tenant_id: str
    run_id: str
    decision_id: str
    schema_bundle_id: str
    producer_id: str
    producer_sequence: int
    producer_signature: bytes
    signing_key_id: str
    predicted_a_tokens: int
    predicted_b_tokens: int
    predicted_c_tokens: int
    reserved_strategy: str
    prediction_strategy_used: str
    prediction_policy_used: str
    tokenizer_tier: str
    tokenizer_version_id: str
    prediction_confidence: float
    prediction_sample_size: int
    cold_start_layer_used: str
    run_projection_at_decision_atomic: int
    run_predicted_remaining_steps: int
    run_steps_completed_so_far: int
    actual_input_tokens: int
    actual_output_tokens: int
    delta_b_ratio: float
    delta_c_ratio: float
    def __init__(self, specversion: _Optional[str] = ..., type: _Optional[str] = ..., source: _Optional[str] = ..., id: _Optional[str] = ..., time: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ..., datacontenttype: _Optional[str] = ..., data: _Optional[bytes] = ..., tenant_id: _Optional[str] = ..., run_id: _Optional[str] = ..., decision_id: _Optional[str] = ..., schema_bundle_id: _Optional[str] = ..., producer_id: _Optional[str] = ..., producer_sequence: _Optional[int] = ..., producer_signature: _Optional[bytes] = ..., signing_key_id: _Optional[str] = ..., predicted_a_tokens: _Optional[int] = ..., predicted_b_tokens: _Optional[int] = ..., predicted_c_tokens: _Optional[int] = ..., reserved_strategy: _Optional[str] = ..., prediction_strategy_used: _Optional[str] = ..., prediction_policy_used: _Optional[str] = ..., tokenizer_tier: _Optional[str] = ..., tokenizer_version_id: _Optional[str] = ..., prediction_confidence: _Optional[float] = ..., prediction_sample_size: _Optional[int] = ..., cold_start_layer_used: _Optional[str] = ..., run_projection_at_decision_atomic: _Optional[int] = ..., run_predicted_remaining_steps: _Optional[int] = ..., run_steps_completed_so_far: _Optional[int] = ..., actual_input_tokens: _Optional[int] = ..., actual_output_tokens: _Optional[int] = ..., delta_b_ratio: _Optional[float] = ..., delta_c_ratio: _Optional[float] = ...) -> None: ...

class SchemaBundleRef(_message.Message):
    __slots__ = ("schema_bundle_id", "schema_bundle_hash", "canonical_schema_version")
    SCHEMA_BUNDLE_ID_FIELD_NUMBER: _ClassVar[int]
    SCHEMA_BUNDLE_HASH_FIELD_NUMBER: _ClassVar[int]
    CANONICAL_SCHEMA_VERSION_FIELD_NUMBER: _ClassVar[int]
    schema_bundle_id: str
    schema_bundle_hash: bytes
    canonical_schema_version: str
    def __init__(self, schema_bundle_id: _Optional[str] = ..., schema_bundle_hash: _Optional[bytes] = ..., canonical_schema_version: _Optional[str] = ...) -> None: ...

class ContractBundleRef(_message.Message):
    __slots__ = ("bundle_id", "bundle_hash", "bundle_signature", "signing_key_id")
    BUNDLE_ID_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_HASH_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_SIGNATURE_FIELD_NUMBER: _ClassVar[int]
    SIGNING_KEY_ID_FIELD_NUMBER: _ClassVar[int]
    bundle_id: str
    bundle_hash: bytes
    bundle_signature: bytes
    signing_key_id: str
    def __init__(self, bundle_id: _Optional[str] = ..., bundle_hash: _Optional[bytes] = ..., bundle_signature: _Optional[bytes] = ..., signing_key_id: _Optional[str] = ...) -> None: ...

class Idempotency(_message.Message):
    __slots__ = ("key", "request_hash")
    KEY_FIELD_NUMBER: _ClassVar[int]
    REQUEST_HASH_FIELD_NUMBER: _ClassVar[int]
    key: str
    request_hash: bytes
    def __init__(self, key: _Optional[str] = ..., request_hash: _Optional[bytes] = ...) -> None: ...
