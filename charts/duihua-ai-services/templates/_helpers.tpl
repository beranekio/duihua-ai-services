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

{{- define "duihua.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "duihua.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
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
{{- if ne (include "duihua.responsesApiStore.enabled" .) "true" -}}
{{- else if not .Values.backgroundWorker.enabled -}}
{{- else if eq (dig "backgroundJobs" "enabled" true .Values.gateway.responsesApiStore) false -}}
{{- else -}}
true
{{- end -}}
{{- end -}}

{{- define "duihua.background.streamKey" -}}
{{- $legacyStreamKey := dig "backgroundJobs" "streamKey" "" .Values.gateway.responsesApiStore -}}
{{- if $legacyStreamKey -}}
{{- $legacyStreamKey -}}
{{- else -}}
{{- .Values.backgroundWorker.streamKey -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.staleSeconds" -}}
{{- $legacyStaleSeconds := dig "backgroundJobs" "staleSeconds" "" .Values.gateway.responsesApiStore -}}
{{- if $legacyStaleSeconds -}}
{{- $legacyStaleSeconds -}}
{{- else -}}
{{- .Values.backgroundWorker.staleSeconds -}}
{{- end -}}
{{- end -}}