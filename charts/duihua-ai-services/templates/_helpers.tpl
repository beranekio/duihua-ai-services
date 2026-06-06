{{- define "duihua.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "duihua.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name (include "duihua.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responsesApiStore.enabled" -}}
{{- if hasKey .Values.gateway.responsesApiStore "enabled" -}}
{{- .Values.gateway.responsesApiStore.enabled -}}
{{- else -}}
{{- .Values.inference.responsesApiStore.enabled | default false -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreUrl" -}}
{{- if .Values.valkey.enabled -}}
redis://{{ include "duihua.fullname" . }}-valkey:{{ .Values.valkey.service.port }}
{{- else -}}
{{- .Values.gateway.env.responseIdStoreUrl -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.enabled" -}}
{{- if and (eq (include "duihua.responsesApiStore.enabled" .) "true") .Values.backgroundWorker.enabled -}}
true
{{- end -}}
{{- end -}}