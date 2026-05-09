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
{{- if not (contains "/" $repo) -}}
{{- printf "%s/%s:%s" .global.imageRegistry $repo .image.tag -}}
{{- else -}}
{{- printf "%s:%s" $repo .image.tag -}}
{{- end -}}
{{- end -}}
