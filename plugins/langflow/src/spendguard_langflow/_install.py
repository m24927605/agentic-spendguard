"""``spendguard-langflow-install`` CLI -- copy the component into LANGFLOW_COMPONENTS_PATH.

Usage::

    spendguard-langflow-install --target /path/to/components_dir
    spendguard-langflow-install --target /path/to/components_dir --force

Safety:

* Refuses targets matching system paths (``/usr``, ``/bin``, ``/etc``,
  ``/System``) per INV-8.
* Refuses to overwrite an existing file unless ``--force``.
* Auto-creates parent directories.
* Does NOT touch ``$LANGFLOW_COMPONENTS_PATH`` itself -- operator names
  the target explicitly so the tool never guesses where to write.

Copies BOTH the Python component (``spendguard_chat_model_wrapper.py``,
synthesized as a thin shim that re-imports the installed package) AND
the Langflow metadata YAML (``spendguard_chat_model_wrapper.yaml``).
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
import textwrap
from pathlib import Path


_SYSTEM_PATH_PREFIXES = (
    "/usr",
    "/bin",
    "/etc",
    "/System",
    "/sbin",
    "/Library/System",
    # macOS resolves /etc -> /private/etc and /sbin -> /private/sbin via
    # symlinks. Catch the resolved paths too so the refusal fires
    # regardless of which form the operator passes in.
    # We deliberately do NOT include /private/var: macOS TMPDIR is
    # under /private/var/folders and pytest fixtures legitimately use
    # it. Operators with custom /private/var/lib trees can self-impose
    # their own --target check.
    "/private/etc",
    "/private/sbin",
)
"""Paths the installer refuses to write to.

Per review-standards §3.6 / INV-8: operators MUST point at their own
``LANGFLOW_COMPONENTS_PATH`` tree. Writing to ``/usr`` etc. is a
supply-chain footgun -- the script refuses with a clear error.
"""

_COMPONENT_SHIM = textwrap.dedent(
    '''\
    """Vendored entrypoint installed by ``spendguard-langflow-install``.

    This file re-exports the SpendGuard Langflow component so Langflow's
    component loader picks it up while keeping the implementation in
    the pip-installed ``spendguard_langflow`` package -- pip upgrades
    propagate without re-running the install script.
    """

    from spendguard_langflow.component import SpendGuardChatModelWrapper  # noqa: F401

    __all__ = ["SpendGuardChatModelWrapper"]
    '''
)
"""Synthetic vendored shim file content.

Langflow's component loader walks ``LANGFLOW_COMPONENTS_PATH`` for
``.py`` files. We drop a small file that re-imports the pip-installed
class so upgrades stay surgical.
"""


def _is_system_path(target: Path) -> bool:
    """Reject obvious system locations regardless of write permissions.

    Checks both the literal supplied path AND its
    :py:meth:`Path.resolve` form so symlinked roots (e.g. macOS
    ``/etc`` -> ``/private/etc``) are caught consistently.
    """
    literal = str(target)
    candidates = [literal]
    try:
        candidates.append(str(target.resolve(strict=False)))
    except OSError:
        # Resolve can fail on some platforms when the path doesn't
        # exist. The literal-path check is enough in that case.
        pass
    for resolved in candidates:
        for prefix in _SYSTEM_PATH_PREFIXES:
            if resolved == prefix or resolved.startswith(prefix + os.sep):
                return True
    return False


def _metadata_yaml_source() -> Path:
    """Locate the bundled metadata YAML inside the installed package."""
    # __file__ is .../spendguard_langflow/_install.py.
    # metadata YAML ships alongside via the wheel's package_data.
    pkg_dir = Path(__file__).resolve().parent
    candidate = pkg_dir / "metadata" / "spendguard_chat_model_wrapper.yaml"
    if candidate.exists():
        return candidate
    # Editable install / source checkout fallback: walk up to repo
    # plugins/langflow/metadata/.
    repo_candidate = (
        pkg_dir.parent.parent / "metadata" / "spendguard_chat_model_wrapper.yaml"
    )
    if repo_candidate.exists():
        return repo_candidate
    raise FileNotFoundError(
        "Bundled metadata YAML not found. Reinstall the package via "
        "pip install spendguard-langflow-component."
    )


def install(target: Path, *, force: bool = False) -> tuple[Path, Path]:
    """Copy component + metadata into ``target``.

    Args:
        target: directory to write into. Created if missing.
        force: overwrite existing files instead of refusing.

    Returns:
        Paths of (component_file, metadata_file) actually written.

    Raises:
        FileExistsError: target file exists and ``force is False``.
        PermissionError: target is a refused system path (INV-8).
    """
    if _is_system_path(target):
        raise PermissionError(
            f"Refusing to install into system path {target!s}. Point "
            "--target at your own LANGFLOW_COMPONENTS_PATH directory."
        )
    target.mkdir(parents=True, exist_ok=True)
    component_dst = target / "spendguard_chat_model_wrapper.py"
    metadata_dst = target / "spendguard_chat_model_wrapper.yaml"

    if not force:
        if component_dst.exists():
            raise FileExistsError(
                f"{component_dst!s} exists. Re-run with --force to overwrite."
            )
        if metadata_dst.exists():
            raise FileExistsError(
                f"{metadata_dst!s} exists. Re-run with --force to overwrite."
            )

    component_dst.write_text(_COMPONENT_SHIM, encoding="utf-8")
    metadata_src = _metadata_yaml_source()
    shutil.copy2(metadata_src, metadata_dst)
    return component_dst, metadata_dst


def _parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="spendguard-langflow-install",
        description=(
            "Install the SpendGuard Langflow custom component into a "
            "Langflow components directory."
        ),
    )
    p.add_argument(
        "--target",
        required=True,
        type=Path,
        help=(
            "Directory to install into (typically your "
            "$LANGFLOW_COMPONENTS_PATH)."
        ),
    )
    p.add_argument(
        "--force",
        action="store_true",
        help="Overwrite existing files if present.",
    )
    return p


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        component_dst, metadata_dst = install(args.target, force=args.force)
    except FileExistsError as exc:
        print(f"spendguard-langflow-install: {exc}", file=sys.stderr)
        return 2
    except PermissionError as exc:
        print(f"spendguard-langflow-install: {exc}", file=sys.stderr)
        return 3
    except FileNotFoundError as exc:
        print(f"spendguard-langflow-install: {exc}", file=sys.stderr)
        return 4
    print(
        f"spendguard-langflow-install: installed\n"
        f"  component: {component_dst}\n"
        f"  metadata : {metadata_dst}"
    )
    return 0


__all__ = ["install", "main"]


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
