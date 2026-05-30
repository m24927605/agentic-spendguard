"""Unit tests for ``spendguard.run_plan`` (Signal 3 decorator).

Spec ref ``run-cost-projector-spec-v1alpha1.md`` §5.
"""

from __future__ import annotations

import asyncio

import pytest

from spendguard import (
    RunPlan,
    current_run_plan,
    with_run_plan,
)


class TestRunPlanDataclass:
    def test_planned_steps_hint_sum(self) -> None:
        assert RunPlan(planned_calls=3, planned_tools=2).planned_steps_hint == 5

    def test_zero_tools(self) -> None:
        assert RunPlan(planned_calls=5, planned_tools=0).planned_steps_hint == 5

    def test_frozen(self) -> None:
        plan = RunPlan(planned_calls=1, planned_tools=1)
        with pytest.raises(Exception):  # FrozenInstanceError or AttributeError
            plan.planned_calls = 10  # type: ignore[misc]


class TestSyncDecorator:
    def test_sync_function_runs(self) -> None:
        @with_run_plan(planned_calls=3, planned_tools=2)
        def sync_fn() -> int:
            assert current_run_plan() is not None
            return 42

        result = sync_fn()
        assert result == 42
        # Plan cleared after exit
        assert current_run_plan() is None

    def test_sync_returns_function_value(self) -> None:
        @with_run_plan(planned_calls=1)
        def sync_fn(x: int, y: int) -> int:
            return x + y

        assert sync_fn(3, 4) == 7

    def test_sync_plan_visible_inside(self) -> None:
        captured: list[RunPlan | None] = []

        @with_run_plan(planned_calls=8, planned_tools=2)
        def sync_fn() -> None:
            captured.append(current_run_plan())

        sync_fn()
        assert captured[0] is not None
        assert captured[0].planned_calls == 8
        assert captured[0].planned_tools == 2
        assert captured[0].planned_steps_hint == 10


class TestAsyncDecorator:
    @pytest.mark.asyncio
    async def test_async_function_runs(self) -> None:
        @with_run_plan(planned_calls=3, planned_tools=2)
        async def async_fn() -> int:
            assert current_run_plan() is not None
            return 42

        result = await async_fn()
        assert result == 42
        assert current_run_plan() is None

    @pytest.mark.asyncio
    async def test_async_returns_function_value(self) -> None:
        @with_run_plan(planned_calls=5)
        async def async_fn(x: int) -> int:
            return x * 2

        assert await async_fn(7) == 14

    @pytest.mark.asyncio
    async def test_async_plan_visible_inside(self) -> None:
        captured: list[RunPlan | None] = []

        @with_run_plan(planned_calls=4, planned_tools=1)
        async def async_fn() -> None:
            captured.append(current_run_plan())

        await async_fn()
        assert captured[0] is not None
        assert captured[0].planned_steps_hint == 5

    @pytest.mark.asyncio
    async def test_async_plan_visible_across_await(self) -> None:
        """Context-var survives await boundaries."""
        captured: list[RunPlan | None] = []

        @with_run_plan(planned_calls=2)
        async def async_fn() -> None:
            captured.append(current_run_plan())
            await asyncio.sleep(0)  # yield to event loop
            captured.append(current_run_plan())

        await async_fn()
        assert captured[0] is not None
        assert captured[1] is not None
        assert captured[0].planned_calls == 2
        assert captured[1].planned_calls == 2


class TestNestedDecorator:
    """Outer plan wins per spec §5.2."""

    def test_sync_nested_outer_wins(self) -> None:
        captured: list[RunPlan | None] = []

        @with_run_plan(planned_calls=10, planned_tools=5)
        def outer() -> None:
            captured.append(current_run_plan())

            @with_run_plan(planned_calls=1, planned_tools=1)
            def inner() -> None:
                captured.append(current_run_plan())

            inner()
            captured.append(current_run_plan())

        outer()
        # 3 captures: outer entry, inner entry (still sees outer), outer after inner
        assert len(captured) == 3
        assert all(p is not None for p in captured)
        # All three see the outer plan (10+5)
        assert all(p.planned_steps_hint == 15 for p in captured)  # type: ignore[union-attr]

    @pytest.mark.asyncio
    async def test_async_nested_outer_wins(self) -> None:
        captured: list[RunPlan | None] = []

        @with_run_plan(planned_calls=10)
        async def outer() -> None:
            captured.append(current_run_plan())

            @with_run_plan(planned_calls=1)
            async def inner() -> None:
                captured.append(current_run_plan())

            await inner()
            captured.append(current_run_plan())

        await outer()
        assert all(p is not None for p in captured)
        assert all(p.planned_calls == 10 for p in captured)  # type: ignore[union-attr]


class TestDecoratorValidation:
    def test_negative_planned_calls_rejected(self) -> None:
        with pytest.raises(TypeError, match="non-negative"):
            with_run_plan(planned_calls=-1)

    def test_non_int_planned_calls_rejected(self) -> None:
        with pytest.raises(TypeError, match="non-negative"):
            with_run_plan(planned_calls="five")  # type: ignore[arg-type]

    def test_negative_planned_tools_rejected(self) -> None:
        with pytest.raises(TypeError, match="non-negative"):
            with_run_plan(planned_calls=5, planned_tools=-1)

    def test_planned_tools_default_zero(self) -> None:
        @with_run_plan(planned_calls=3)
        def fn() -> RunPlan | None:
            return current_run_plan()

        plan = fn()
        assert plan is not None
        assert plan.planned_tools == 0

    def test_non_callable_target_rejected(self) -> None:
        decorator = with_run_plan(planned_calls=5)
        with pytest.raises(TypeError, match="target must be callable"):
            decorator("not a function")  # type: ignore[arg-type]


class TestCleanup:
    """Context-var is properly cleared even on exceptions."""

    def test_sync_exception_clears_plan(self) -> None:
        @with_run_plan(planned_calls=3)
        def fn() -> None:
            raise ValueError("boom")

        with pytest.raises(ValueError):
            fn()
        # Plan must be cleared even after exception
        assert current_run_plan() is None

    @pytest.mark.asyncio
    async def test_async_exception_clears_plan(self) -> None:
        @with_run_plan(planned_calls=3)
        async def fn() -> None:
            raise ValueError("boom")

        with pytest.raises(ValueError):
            await fn()
        assert current_run_plan() is None


class TestCurrentRunPlan:
    def test_returns_none_outside_decorated_frame(self) -> None:
        assert current_run_plan() is None

    def test_returns_plan_inside_decorated_frame(self) -> None:
        @with_run_plan(planned_calls=7, planned_tools=3)
        def fn() -> RunPlan | None:
            return current_run_plan()

        plan = fn()
        assert plan is not None
        assert plan.planned_calls == 7
        assert plan.planned_tools == 3
