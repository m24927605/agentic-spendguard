{{/*
Common labels + naming. All resources include
   app.kubernetes.io/name, /component, /instance, /version, /managed-by.
*/}}

{{- define "spendguard.name" -}}
spendguard
{{- end -}}

{{- define "spendguard.fullname" -}}
{{- printf "%s-%s" .Release.Name (include "spendguard.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "spendguard.labels" -}}
app.kubernetes.io/name: {{ include "spendguard.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "spendguard.componentLabels" -}}
{{ include "spendguard.labels" . }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "spendguard.selector" -}}
app.kubernetes.io/name: {{ include "spendguard.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{/*
Container image reference; falls back to imageRegistry global if
the per-service repository doesn't include a registry prefix.
*/}}
{{- define "spendguard.image" -}}
{{- $repo := .image.repository -}}
{{- $tag := toString .image.tag -}}
{{- $first := first (splitList "/" $repo) -}}
{{- $qualifiedRepo := "" -}}
{{- if or (contains "." $first) (contains ":" $first) (eq $first "localhost") -}}
{{- $qualifiedRepo = $repo -}}
{{- else -}}
{{- $qualifiedRepo = printf "%s/%s" .global.imageRegistry $repo -}}
{{- end -}}
{{- if hasPrefix "@sha256:" $tag -}}
{{- printf "%s%s" $qualifiedRepo $tag -}}
{{- else -}}
{{- printf "%s:%s" $qualifiedRepo $tag -}}
{{- end -}}
{{- end -}}

{{/*
Render an env.valueFrom.secretKeyRef for database URLs.

HARDEN_03 / issue #145: Postgres URLs contain credentials and must not
land as literal values in rendered Kubernetes manifests. Operators
pre-create .Values.postgres.existingSecret with one key per logical DB
URL, and workloads reference the key by name.
*/}}
{{- define "spendguard.postgresSecretRef" -}}
valueFrom:
  secretKeyRef:
    name: {{ .root.Values.postgres.existingSecret | quote }}
    key: {{ .key | quote }}
{{- end -}}

{{/*
Container security baseline shared by production workloads.
*/}}
{{- define "spendguard.containerSecurityContext" -}}
readOnlyRootFilesystem: true
allowPrivilegeEscalation: false
capabilities:
  drop: ["ALL"]
{{- end -}}

{{/*
Pod security baseline shared by production workloads.
*/}}
{{- define "spendguard.podSecurityContext" -}}
runAsNonRoot: true
runAsUser: 65532
runAsGroup: 65532
fsGroup: 65532
seccompProfile:
  type: RuntimeDefault
{{- end -}}

{{/*
Validate a first-party production image tag. Values may be either immutable
release semver (vN.N.N with optional prerelease) or a digest suffix.
*/}}
{{- define "spendguard.validateProductionImageTag" -}}
{{- if eq .root.Values.chart.profile "production" -}}
{{- $tag := toString .tag -}}
{{- if eq $tag "" -}}
{{- fail (printf "%s.image.tag is required under chart.profile=production" .name) -}}
{{- end -}}
{{- if not (or (regexMatch "@sha256:[0-9a-f]{64}$" $tag) (regexMatch "^v?[0-9]+\\.[0-9]+\\.[0-9]+(-[a-z0-9.]+)?$" $tag)) -}}
{{- fail (printf "%s.image.tag=%q must be a semver tag (vN.N.N) or digest-pinned (@sha256:HEX) under chart.profile=production" .name $tag) -}}
{{- end -}}
{{- end -}}
{{- end -}}
