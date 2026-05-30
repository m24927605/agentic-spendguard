-- HARDEN_01: mirror prediction extension fields from the signed
-- CloudEvent envelope into audit_outbox's first-class columns.
--
-- SLICE_08-15 retrospective review found that the ledger handlers now
-- preserve the full signed CloudEvent JSON in cloudevent_payload, but
-- the stored procedures still insert only the base audit_outbox columns.
-- This BEFORE INSERT trigger closes that gap centrally without editing
-- every historical post_* stored procedure. It also makes the
-- tokenizer_version_id FK in 0048 effective for new decision rows.
--
-- The trigger only fills NULL SQL columns. Future producers that write
-- first-class columns directly keep working; malformed or unknown UUID
-- values still fail at INSERT via the UUID cast / FK.

CREATE OR REPLACE FUNCTION mirror_audit_outbox_prediction_columns()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_projection NUMERIC(38,0) := COALESCE(
        NULLIF(NEW.cloudevent_payload->>'run_projection_at_decision_atomic', '')::NUMERIC(38,0),
        0
    );
    v_remaining INT := COALESCE(
        NULLIF(NEW.cloudevent_payload->>'run_predicted_remaining_steps', '')::INT,
        -1
    );
    v_steps BIGINT := COALESCE(
        NULLIF(NEW.cloudevent_payload->>'run_steps_completed_so_far', '')::BIGINT,
        0
    );
    v_data JSONB := NULL;
    v_projector_unreachable BOOLEAN := (v_projection = 0 AND v_remaining = -1 AND v_steps = 0);
    v_projector_absent_default BOOLEAN := (v_projection = 0 AND v_remaining = 0 AND v_steps = 0);
    v_no_projector BOOLEAN := (v_projector_unreachable OR v_projector_absent_default);
BEGIN
    IF NEW.cloudevent_payload IS NULL THEN
        RETURN NEW;
    END IF;

    IF NEW.event_type = 'spendguard.audit.decision' THEN
        NEW.predicted_a_tokens := COALESCE(
            NEW.predicted_a_tokens,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'predicted_a_tokens', '')::BIGINT, 0)
        );
        NEW.predicted_b_tokens := COALESCE(
            NEW.predicted_b_tokens,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'predicted_b_tokens', '')::BIGINT, 0)
        );
        NEW.predicted_c_tokens := COALESCE(
            NEW.predicted_c_tokens,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'predicted_c_tokens', '')::BIGINT, 0)
        );
        NEW.reserved_strategy := COALESCE(
            NEW.reserved_strategy,
            NULLIF(NEW.cloudevent_payload->>'reserved_strategy', '')
        );
        NEW.prediction_strategy_used := COALESCE(
            NEW.prediction_strategy_used,
            NULLIF(NEW.cloudevent_payload->>'prediction_strategy_used', '')
        );
        NEW.prediction_policy_used := COALESCE(
            NEW.prediction_policy_used,
            NULLIF(NEW.cloudevent_payload->>'prediction_policy_used', '')
        );
        NEW.tokenizer_tier := COALESCE(
            NEW.tokenizer_tier,
            NULLIF(NEW.cloudevent_payload->>'tokenizer_tier', '')
        );
        NEW.tokenizer_version_id := COALESCE(
            NEW.tokenizer_version_id,
            NULLIF(NEW.cloudevent_payload->>'tokenizer_version_id', '')::UUID
        );
        NEW.prediction_confidence := COALESCE(
            NEW.prediction_confidence,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'prediction_confidence', '')::NUMERIC(4,3), 0)
        );
        NEW.prediction_sample_size := COALESCE(
            NEW.prediction_sample_size,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'prediction_sample_size', '')::BIGINT, 0)
        );
        NEW.cold_start_layer_used := COALESCE(
            NEW.cold_start_layer_used,
            NULLIF(NEW.cloudevent_payload->>'cold_start_layer_used', '')
        );

        NEW.run_projection_at_decision_atomic := COALESCE(
            NEW.run_projection_at_decision_atomic,
            CASE WHEN v_no_projector THEN NULL ELSE v_projection END
        );
        NEW.run_predicted_remaining_steps := COALESCE(
            NEW.run_predicted_remaining_steps,
            CASE WHEN v_no_projector OR v_remaining = -1 THEN NULL ELSE v_remaining END
        );
        NEW.run_steps_completed_so_far := COALESCE(
            NEW.run_steps_completed_so_far,
            CASE WHEN v_no_projector THEN NULL ELSE v_steps END
        );
    ELSIF NEW.event_type = 'spendguard.audit.outcome' THEN
        IF NULLIF(NEW.cloudevent_payload->>'data_b64', '') IS NOT NULL THEN
            v_data := convert_from(
                decode(NEW.cloudevent_payload->>'data_b64', 'base64'),
                'UTF8'
            )::JSONB;
        END IF;

        NEW.actual_input_tokens := COALESCE(
            NEW.actual_input_tokens,
            CASE
                WHEN v_data ? 'actual_input_tokens' THEN
                    CASE
                        WHEN NULLIF(v_data->>'actual_input_tokens', '')::BIGINT >= 0
                        THEN NULLIF(v_data->>'actual_input_tokens', '')::BIGINT
                        ELSE NULL
                    END
                WHEN NULLIF(NEW.cloudevent_payload->>'actual_input_tokens', '')::BIGINT > 0
                    THEN NULLIF(NEW.cloudevent_payload->>'actual_input_tokens', '')::BIGINT
                ELSE NULL
            END
        );
        NEW.actual_output_tokens := COALESCE(
            NEW.actual_output_tokens,
            CASE
                WHEN v_data ? 'actual_output_tokens' THEN
                    CASE
                        WHEN NULLIF(v_data->>'actual_output_tokens', '')::BIGINT >= 0
                        THEN NULLIF(v_data->>'actual_output_tokens', '')::BIGINT
                        ELSE NULL
                    END
                WHEN NULLIF(NEW.cloudevent_payload->>'actual_output_tokens', '')::BIGINT > 0
                    THEN NULLIF(NEW.cloudevent_payload->>'actual_output_tokens', '')::BIGINT
                ELSE NULL
            END
        );
        NEW.delta_b_ratio := COALESCE(
            NEW.delta_b_ratio,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'delta_b_ratio', '')::REAL, 0.0::REAL)
        );
        NEW.delta_c_ratio := COALESCE(
            NEW.delta_c_ratio,
            NULLIF(NULLIF(NEW.cloudevent_payload->>'delta_c_ratio', '')::REAL, 0.0::REAL)
        );
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_outbox_prediction_payload_mirror ON audit_outbox;
CREATE TRIGGER audit_outbox_prediction_payload_mirror
    BEFORE INSERT ON audit_outbox
    FOR EACH ROW
    EXECUTE FUNCTION mirror_audit_outbox_prediction_columns();

COMMENT ON FUNCTION mirror_audit_outbox_prediction_columns() IS
    'HARDEN_01: BEFORE INSERT mirror from audit_outbox.cloudevent_payload prediction extension fields into first-class audit_outbox columns so FK/check/index invariants apply to legacy stored-procedure insert paths.';
