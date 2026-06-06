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

{{- define "duihua.responseIdStoreAddress" -}}
{{- if .Values.valkey.enabled -}}
{{- printf "%s-valkey.%s.svc.cluster.local:%v" (include "duihua.fullname" .) .Release.Namespace .Values.valkey.service.port -}}
{{- else -}}
{{- (urlParse .Values.gateway.env.responseIdStoreUrl).host -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreTLSEnabled" -}}
{{- if .Values.valkey.enabled -}}
false
{{- else -}}
{{- eq (urlParse .Values.gateway.env.responseIdStoreUrl).scheme "rediss" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreDatabaseIndex" -}}
{{- if .Values.valkey.enabled -}}
0
{{- else -}}
{{- $path := (urlParse .Values.gateway.env.responseIdStoreUrl).path | default "" -}}
{{- trimPrefix "/" $path | default "0" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.minReplicas" -}}
{{- $replicas := get .Values.backgroundWorker.autoscaling "replicas" | default dict -}}
{{- if hasKey $replicas "min" -}}
{{- $replicas.min -}}
{{- else -}}
1
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.maxReplicas" -}}
{{- $replicas := get .Values.backgroundWorker.autoscaling "replicas" | default dict -}}
{{- if hasKey $replicas "max" -}}
{{- $replicas.max -}}
{{- else -}}
4
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.activationLagCount" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- if hasKey $autoscaling "activationLagCount" -}}
{{- $autoscaling.activationLagCount -}}
{{- else -}}
0
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
{{- $backgroundJobs := get .Values.gateway.responsesApiStore "backgroundJobs" | default dict -}}
{{- if hasKey $backgroundJobs "streamKey" -}}
{{- get $backgroundJobs "streamKey" -}}
{{- else -}}
{{- .Values.backgroundWorker.streamKey -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.staleSeconds" -}}
{{- $backgroundJobs := get .Values.gateway.responsesApiStore "backgroundJobs" | default dict -}}
{{- if hasKey $backgroundJobs "staleSeconds" -}}
{{- get $backgroundJobs "staleSeconds" -}}
{{- else -}}
{{- .Values.backgroundWorker.staleSeconds -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.enabled" -}}
{{- if ne (include "duihua.background.enabled" .) "true" -}}
{{- else if not .Values.backgroundWorker.autoscaling.enabled -}}
{{- else -}}
true
{{- end -}}
{{- end -}}