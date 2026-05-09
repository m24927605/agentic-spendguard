PROTO_ROOT := $(realpath $(CURDIR)/../../proto)
OUT_DIR    := $(CURDIR)/src/spendguard/_proto
# ledger.proto is included for the demo's webhook simulator path
# (talks directly to ledger gRPC for Step 8 dev convenience).
# Production adapters do NOT use ledger.proto; sidecar is the only
# ledger client per Stage 2 §3.1.
PROTOS     := \
	$(PROTO_ROOT)/spendguard/common/v1/common.proto \
	$(PROTO_ROOT)/spendguard/sidecar_adapter/v1/adapter.proto \
	$(PROTO_ROOT)/spendguard/ledger/v1/ledger.proto

.PHONY: proto clean lint typecheck test build publish-test publish

# Regenerate Python protobuf + gRPC stubs from the wire spec.
# Output lands under src/spendguard/_proto/spendguard/...
# Generated code uses absolute imports rooted at `spendguard.*`; we
# rewrite them to `spendguard._proto.spendguard.*` so the wheel ships
# a self-contained import tree.
proto:
	@mkdir -p $(OUT_DIR)
	python -m grpc_tools.protoc \
		--proto_path=$(PROTO_ROOT) \
		--python_out=$(OUT_DIR) \
		--grpc_python_out=$(OUT_DIR) \
		--pyi_out=$(OUT_DIR) \
		$(PROTOS)
	@find $(OUT_DIR) -type d -exec sh -c 'touch "$$1/__init__.py"' _ {} \;
	@# Rewrite both `from spendguard.X import` and `import spendguard.X`
	@# patterns to the namespaced location.
	@find $(OUT_DIR) \( -name '*.py' -o -name '*.pyi' \) \
		-exec sed -i.bak \
			-e 's|^from spendguard\.|from spendguard._proto.spendguard.|g' \
			-e 's|^import spendguard\.|import spendguard._proto.spendguard.|g' \
			{} \;
	@find $(OUT_DIR) -name '*.bak' -delete

clean:
	rm -rf $(OUT_DIR) dist build *.egg-info

lint:
	ruff check src

typecheck:
	mypy src

test:
	pytest -q

# Build sdist + wheel for publishing.
build: clean proto
	python -m build

# Publish to test.pypi.org for verification.
publish-test: build
	python -m twine check dist/*
	python -m twine upload --repository testpypi dist/*

# Publish to PyPI (real).
publish: build
	python -m twine check dist/*
	python -m twine upload dist/*
