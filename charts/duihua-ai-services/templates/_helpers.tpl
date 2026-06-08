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

{{- define "duihua.responsesApiStoreService.fullname" -}}
{{- printf "%s-responses-api-store" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "duihua.responsesApiStoreService.enabled" -}}
{{- if .Values.responsesApiStoreService.enabled -}}
true
{{- else -}}
false
{{- end -}}
{{- end -}}

{{- define "duihua.responsesApiStoreEndpoint" -}}
{{- if .Values.gateway.responsesApiStore.endpoint -}}
{{- .Values.gateway.responsesApiStore.endpoint -}}
{{- else -}}
{{- printf "http://%s:%d" (include "duihua.responsesApiStoreService.fullname" .) (int (index .Values "responses-api-store").grpc.port) -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.streamKey" -}}
{{- if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- (index .Values "responses-api-store").backgroundQueue.streamKey -}}
{{- else -}}
{{- $backgroundJobs := get .Values.gateway.responsesApiStore "backgroundJobs" | default dict -}}
{{- if hasKey $backgroundJobs "streamKey" -}}
{{- get $backgroundJobs "streamKey" -}}
{{- else -}}
{{- .Values.backgroundWorker.streamKey | default "responses-api-store:background" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreAddress" -}}
{{- if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- $valkey := (index .Values "responses-api-store").valkey -}}
{{- printf "%s-responses-api-store-valkey.%s.svc.cluster.local:%v" .Release.Name .Release.Namespace $valkey.service.port -}}
{{- else if .Values.gateway.responsesApiStore.redisAddress -}}
{{- .Values.gateway.responsesApiStore.redisAddress -}}
{{- else -}}
{{- fail "gateway.responsesApiStore.redisAddress must be set when responsesApiStoreService.enabled is false" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreTLSEnabled" -}}
{{- if .Values.gateway.responsesApiStore.redisEnableTLS -}}
true
{{- else -}}
false
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreDatabaseIndex" -}}
{{- .Values.gateway.responsesApiStore.redisDatabaseIndex | default "0" -}}
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

{{- define "duihua.background.autoscaling.driver" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- $driver := $autoscaling.driver | default "store-metrics" -}}
{{- $driver -}}
{{- end -}}

{{- define "duihua.responsesApiStoreMetricsPort" -}}
{{- $metrics := (index .Values "responses-api-store").metrics | default dict -}}
{{- if hasKey $metrics "port" -}}
{{- $metrics.port -}}
{{- else -}}
8080
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.metricsUrl" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- if $autoscaling.metricsUrl -}}
{{- $autoscaling.metricsUrl -}}
{{- else if .Values.gateway.responsesApiStore.metricsUrl -}}
{{- .Values.gateway.responsesApiStore.metricsUrl -}}
{{- else if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- $metrics := (index .Values "responses-api-store").metrics | default dict -}}
{{- if eq ($metrics.enabled | default true) false -}}
{{- fail "responses-api-store.metrics.enabled must be true when backgroundWorker.autoscaling.driver is store-metrics (or set backgroundWorker.autoscaling.metricsUrl)" -}}
{{- end -}}
{{- printf "http://%s.%s.svc.cluster.local:%v/metrics/background-queue?consumer_group=%s" (include "duihua.responsesApiStoreService.fullname" .) .Release.Namespace (include "duihua.responsesApiStoreMetricsPort" .) .Values.backgroundWorker.consumerGroup -}}
{{- else -}}
{{- fail "backgroundWorker.autoscaling.metricsUrl must be set when responsesApiStoreService.enabled is false and backgroundWorker.autoscaling.driver is store-metrics" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.jobsPerReplica" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- if hasKey $autoscaling "jobsPerReplica" -}}
{{- $autoscaling.jobsPerReplica -}}
{{- else if hasKey $autoscaling "lagCount" -}}
{{- $autoscaling.lagCount -}}
{{- else -}}
5
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.activationTargetValue" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- if hasKey $autoscaling "activationTargetValue" -}}
{{- $autoscaling.activationTargetValue -}}
{{- else if hasKey $autoscaling "activationLagCount" -}}
{{- $autoscaling.activationLagCount -}}
{{- else -}}
0
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.activationLagCount" -}}
{{- include "duihua.background.autoscaling.activationTargetValue" . -}}
{{- end -}}

{{- define "duihua.background.autoscaling.scaledownPeriod" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- if hasKey $autoscaling "scaledownPeriod" -}}
{{- $autoscaling.scaledownPeriod -}}
{{- else -}}
300
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

{{- define "duihua.background.queueBootstrap.enabled" -}}
{{- end -}}