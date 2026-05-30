from google.protobuf import timestamp_pb2 as _timestamp_pb2
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from typing import ClassVar as _ClassVar, Mapping as _Mapping, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class PredictRequest(_message.Message):
    __slots__ = ("spendguard_call_id", "tenant_id", "model", "agent_id", "prompt_class", "input_tokens", "max_tokens_requested", "classifier_version", "prompt_class_fingerprint", "features")
    class ContextFeatures(_message.Message):
        __slots__ = ("conversation_depth", "has_tool_calls", "has_system_message", "num_tool_definitions", "user_role_hint", "request_time")
        CONVERSATION_DEPTH_FIELD_NUMBER: _ClassVar[int]
        HAS_TOOL_CALLS_FIELD_NUMBER: _ClassVar[int]
        HAS_SYSTEM_MESSAGE_FIELD_NUMBER: _ClassVar[int]
        NUM_TOOL_DEFINITIONS_FIELD_NUMBER: _ClassVar[int]
        USER_ROLE_HINT_FIELD_NUMBER: _ClassVar[int]
        REQUEST_TIME_FIELD_NUMBER: _ClassVar[int]
        conversation_depth: int
        has_tool_calls: bool
        has_system_message: bool
        num_tool_definitions: int
        user_role_hint: str
        request_time: _timestamp_pb2.Timestamp
        def __init__(self, conversation_depth: _Optional[int] = ..., has_tool_calls: bool = ..., has_system_message: bool = ..., num_tool_definitions: _Optional[int] = ..., user_role_hint: _Optional[str] = ..., request_time: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...
    SPENDGUARD_CALL_ID_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    MODEL_FIELD_NUMBER: _ClassVar[int]
    AGENT_ID_FIELD_NUMBER: _ClassVar[int]
    PROMPT_CLASS_FIELD_NUMBER: _ClassVar[int]
    INPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    MAX_TOKENS_REQUESTED_FIELD_NUMBER: _ClassVar[int]
    CLASSIFIER_VERSION_FIELD_NUMBER: _ClassVar[int]
    PROMPT_CLASS_FINGERPRINT_FIELD_NUMBER: _ClassVar[int]
    FEATURES_FIELD_NUMBER: _ClassVar[int]
    spendguard_call_id: str
    tenant_id: str
    model: str
    agent_id: str
    prompt_class: str
    input_tokens: int
    max_tokens_requested: int
    classifier_version: str
    prompt_class_fingerprint: str
    features: PredictRequest.ContextFeatures
    def __init__(self, spendguard_call_id: _Optional[str] = ..., tenant_id: _Optional[str] = ..., model: _Optional[str] = ..., agent_id: _Optional[str] = ..., prompt_class: _Optional[str] = ..., input_tokens: _Optional[int] = ..., max_tokens_requested: _Optional[int] = ..., classifier_version: _Optional[str] = ..., prompt_class_fingerprint: _Optional[str] = ..., features: _Optional[_Union[PredictRequest.ContextFeatures, _Mapping]] = ...) -> None: ...

class PredictResponse(_message.Message):
    __slots__ = ("predicted_output_tokens", "confidence", "sample_size", "plugin_version", "feature_hash")
    PREDICTED_OUTPUT_TOKENS_FIELD_NUMBER: _ClassVar[int]
    CONFIDENCE_FIELD_NUMBER: _ClassVar[int]
    SAMPLE_SIZE_FIELD_NUMBER: _ClassVar[int]
    PLUGIN_VERSION_FIELD_NUMBER: _ClassVar[int]
    FEATURE_HASH_FIELD_NUMBER: _ClassVar[int]
    predicted_output_tokens: int
    confidence: float
    sample_size: int
    plugin_version: str
    feature_hash: str
    def __init__(self, predicted_output_tokens: _Optional[int] = ..., confidence: _Optional[float] = ..., sample_size: _Optional[int] = ..., plugin_version: _Optional[str] = ..., feature_hash: _Optional[str] = ...) -> None: ...

class HealthCheckRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class HealthCheckResponse(_message.Message):
    __slots__ = ("status", "plugin_version", "checked_at")
    class Status(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
        __slots__ = ()
        STATUS_UNSPECIFIED: _ClassVar[HealthCheckResponse.Status]
        SERVING: _ClassVar[HealthCheckResponse.Status]
        DEGRADED: _ClassVar[HealthCheckResponse.Status]
        NOT_SERVING: _ClassVar[HealthCheckResponse.Status]
    STATUS_UNSPECIFIED: HealthCheckResponse.Status
    SERVING: HealthCheckResponse.Status
    DEGRADED: HealthCheckResponse.Status
    NOT_SERVING: HealthCheckResponse.Status
    STATUS_FIELD_NUMBER: _ClassVar[int]
    PLUGIN_VERSION_FIELD_NUMBER: _ClassVar[int]
    CHECKED_AT_FIELD_NUMBER: _ClassVar[int]
    status: HealthCheckResponse.Status
    plugin_version: str
    checked_at: _timestamp_pb2.Timestamp
    def __init__(self, status: _Optional[_Union[HealthCheckResponse.Status, str]] = ..., plugin_version: _Optional[str] = ..., checked_at: _Optional[_Union[_timestamp_pb2.Timestamp, _Mapping]] = ...) -> None: ...
