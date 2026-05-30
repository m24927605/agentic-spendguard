"""Reference STUB model for the Strategy C output predictor plugin.

.. warning::

    **STUB MODEL — CUSTOMER MUST REPLACE BEFORE PRODUCTION.**

    This file ships a deliberately minimal sklearn ``LinearRegression``
    trained on 10 hand-crafted rows. Its sole purpose is to exercise
    the gRPC wire surface, conformance corpus, and Dockerfile end-to-
    end. The predicted token counts are *not* useful for real budget
    projection.

    Replace this module with your trained model. Keep the public
    interface stable so the rest of the template (server, backtest,
    conformance test) continues to work:

    - ``StubModel(...).predict(X) -> np.ndarray`` returns one
      predicted token count per row.
    - ``StubModel(...).confidence(X) -> np.ndarray`` returns a
      per-row confidence in ``[0, 1]``.
    - ``StubModel.MODEL_VERSION`` is a short opaque string surfaced
      in ``PredictResponse.plugin_version``.

Customer guidance
-----------------

1. Train on your audit data (per `audit-chain-prediction-extension`
   schema). The backtest harness understands the CSV columns
   ``actual_output_tokens``, ``model``, ``prompt_class``, etc.
2. Pickle / joblib / ONNX whatever you like. The server-side wire is
   independent of the model artifact format.
3. Stay calibrated: track ``actual_output_tokens / predicted_tokens``
   ratio at P50/P95. The backtest harness reports both.
4. Bump ``MODEL_VERSION`` every retrain so SpendGuard's audit can
   correlate drift with retrains.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from sklearn.linear_model import LinearRegression

from feature_extractor import SCHEMA, vectorize_dict


# Token counts that the stub regresses against. Chosen so the resulting
# model produces a believable ~200-1200 token range across the seven
# prompt classes. *None of these are real production numbers.*
_TRAINING_ROWS: list[dict] = [
    # (model, prompt_class, input_tokens, max_tokens_requested,
    #  has_tool_calls, has_system_message, expected_output_tokens)
    dict(
        model="gpt-4o",
        prompt_class="chat_short",
        input_tokens=100,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=True,
        expected=180,
    ),
    dict(
        model="gpt-4o",
        prompt_class="chat_long",
        input_tokens=2000,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=True,
        expected=850,
    ),
    dict(
        model="gpt-4o",
        prompt_class="code_gen",
        input_tokens=800,
        max_tokens_requested=2048,
        has_tool_calls=False,
        has_system_message=True,
        expected=1200,
    ),
    dict(
        model="gpt-4o",
        prompt_class="summarization",
        input_tokens=4000,
        max_tokens_requested=512,
        has_tool_calls=False,
        has_system_message=False,
        expected=420,
    ),
    dict(
        model="gpt-4o",
        prompt_class="rag",
        input_tokens=3000,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=True,
        expected=520,
    ),
    dict(
        model="claude-3-5-sonnet-20240620",
        prompt_class="tool_calling",
        input_tokens=600,
        max_tokens_requested=0,
        has_tool_calls=True,
        has_system_message=True,
        expected=300,
    ),
    dict(
        model="claude-3-5-sonnet-20240620",
        prompt_class="chat_short",
        input_tokens=150,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=True,
        expected=240,
    ),
    dict(
        model="gemini-1.5-pro",
        prompt_class="vision",
        input_tokens=1200,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=False,
        expected=600,
    ),
    dict(
        model="gpt-3.5-turbo",
        prompt_class="chat_short",
        input_tokens=80,
        max_tokens_requested=0,
        has_tool_calls=False,
        has_system_message=True,
        expected=160,
    ),
    dict(
        model="gpt-4o-mini",
        prompt_class="rag",
        input_tokens=2500,
        max_tokens_requested=1024,
        has_tool_calls=False,
        has_system_message=True,
        expected=480,
    ),
]


@dataclass
class StubPrediction:
    """Single-row output of the stub model."""

    predicted_tokens: int
    confidence: float
    sample_size: int


class StubModel:
    """Linear regression on the toy training rows above.

    This class exists so the rest of the template can pretend to call
    a "real" model. Replace its body with your trained predictor.
    """

    MODEL_VERSION: str = "stub-linreg-v0"

    def __init__(self) -> None:
        rows = _TRAINING_ROWS
        X = np.vstack([vectorize_dict(r) for r in rows])
        y = np.asarray([r["expected"] for r in rows], dtype=np.float32)
        model = LinearRegression()
        model.fit(X, y)
        self._model = model
        self._sample_size = len(rows)
        # Sanity-check the schema width matches what we trained on.
        assert X.shape[1] == SCHEMA.width, (
            f"feature dim drift: training data is {X.shape[1]}, schema is {SCHEMA.width}"
        )

    def predict(self, X: np.ndarray) -> np.ndarray:
        """Return predicted output tokens per row (rounded, clipped >= 1)."""
        if X.ndim == 1:
            X = X.reshape(1, -1)
        raw = self._model.predict(X)
        clipped = np.clip(raw, 1.0, None)
        return np.rint(clipped).astype(np.int64)

    def confidence(self, X: np.ndarray) -> np.ndarray:
        """Constant ~0.7 confidence per the docstring contract.

        Replace this with calibrated confidence from your model
        (e.g. ``1 - sigmoid(prediction_residual_zscore)``).
        """
        if X.ndim == 1:
            X = X.reshape(1, -1)
        return np.full((X.shape[0],), 0.7, dtype=np.float32)

    def predict_one(self, x: np.ndarray) -> StubPrediction:
        """Convenience wrapper returning a struct for a single feature vector."""
        return StubPrediction(
            predicted_tokens=int(self.predict(x)[0]),
            confidence=float(self.confidence(x)[0]),
            sample_size=self._sample_size,
        )

    @property
    def sample_size(self) -> int:
        return self._sample_size
