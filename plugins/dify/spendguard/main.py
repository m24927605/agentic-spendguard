"""Dify plugin daemon entrypoint.

The Dify plugin runtime expects `main.py` at the plugin root with a
``Plugin(DifyPluginEnv())`` instance that ``main:plugin.run()`` invokes
(see langgenius/dify-official-plugins reference plugins). Slice 1 ships
this entrypoint; provider + LLM model classes are wired by
``provider/spendguard.yaml`` + ``models/llm/llm.yaml``.
"""

from __future__ import annotations

from dify_plugin import DifyPluginEnv, Plugin

plugin = Plugin(DifyPluginEnv())

if __name__ == "__main__":
    plugin.run()
