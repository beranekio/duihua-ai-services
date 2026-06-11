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

{{- define "duihua.gateway.fullname" -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- if $gateway.fullnameOverride -}}
{{- $gateway.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default "duihua-gateway" $gateway.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.gateway.modelUpstreams" -}}
{{- if not .Values.inference.enabled -}}
{{- else -}}
{{- range $index, $model := .Values.inference.models -}}
{{- if $index }},{{ end -}}
{{- $model.name -}}=http://{{ include "duihua.fullname" $ }}-inference-{{ $index }}-proxy:8080/v1
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responsesApiStore.enabled" -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- if hasKey $gateway.responsesApiStore "enabled" -}}
{{- $gateway.responsesApiStore.enabled -}}
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
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- if $gatewayStore.endpoint -}}
{{- $gatewayStore.endpoint -}}
{{- else if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- $store := get .Values "responses-api-store" | default dict -}}
{{- printf "http://%s:%d" (include "duihua.responsesApiStoreService.fullname" .) (int (dig "grpc" "port" 50051 $store)) -}}
{{- else -}}
{{- fail "duihua-gateway.responsesApiStore.enabled=true requires responsesApiStoreService.enabled=true or duihua-gateway.responsesApiStore.endpoint" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.streamKey" -}}
{{- if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- (index .Values "responses-api-store").backgroundQueue.streamKey -}}
{{- else -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $backgroundJobs := get $gateway.responsesApiStore "backgroundJobs" | default dict -}}
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
{{- else -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- if $gatewayStore.redisAddress -}}
{{- $gatewayStore.redisAddress -}}
{{- else -}}
{{- fail "duihua-gateway.responsesApiStore.redisAddress must be set when responsesApiStoreService.enabled is false" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreTLSEnabled" -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- if $gatewayStore.redisEnableTLS -}}
true
{{- else -}}
false
{{- end -}}
{{- end -}}

{{- define "duihua.responseIdStoreDatabaseIndex" -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- $gatewayStore.redisDatabaseIndex | default "0" -}}
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
{{- $store := get .Values "responses-api-store" | default dict -}}
{{- $metrics := $store.metrics | default dict -}}
{{- if $metrics.listenAddr -}}
{{- $parts := splitList ":" $metrics.listenAddr -}}
{{- last $parts | int -}}
{{- else if hasKey $metrics "port" -}}
{{- $metrics.port -}}
{{- else -}}
8080
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.metricsUrl" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- if $autoscaling.metricsUrl -}}
{{- $autoscaling.metricsUrl -}}
{{- else if $gatewayStore.metricsUrl -}}
{{- $gatewayStore.metricsUrl -}}
{{- else if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- $store := get .Values "responses-api-store" | default dict -}}
{{- $metrics := $store.metrics | default dict -}}
{{- if and (hasKey $metrics "enabled") (not $metrics.enabled) -}}
{{- fail "responses-api-store.metrics.enabled must be true when backgroundWorker.autoscaling.driver is store-metrics (or set backgroundWorker.autoscaling.metricsUrl)" -}}
{{- end -}}
{{- $consumerGroup := .Values.backgroundWorker.consumerGroup | urlquery -}}
{{- printf "http://%s.%s.svc.cluster.local:%v/metrics/background-queue?consumer_group=%s" (include "duihua.responsesApiStoreService.fullname" .) .Release.Namespace (include "duihua.responsesApiStoreMetricsPort" .) $consumerGroup -}}
{{- else -}}
{{- fail "backgroundWorker.autoscaling.metricsUrl must be set when responsesApiStoreService.enabled is false and backgroundWorker.autoscaling.driver is store-metrics" -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.jobsPerReplica" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- $driver := include "duihua.background.autoscaling.driver" . -}}
{{- if eq $driver "redis-streams" -}}
{{- if hasKey $autoscaling "lagCount" -}}
{{- $autoscaling.lagCount -}}
{{- else if hasKey $autoscaling "jobsPerReplica" -}}
{{- $autoscaling.jobsPerReplica -}}
{{- else -}}
5
{{- end -}}
{{- else -}}
{{- if hasKey $autoscaling "jobsPerReplica" -}}
{{- $autoscaling.jobsPerReplica -}}
{{- else if hasKey $autoscaling "lagCount" -}}
{{- $autoscaling.lagCount -}}
{{- else -}}
5
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.autoscaling.activationTargetValue" -}}
{{- $autoscaling := .Values.backgroundWorker.autoscaling | default dict -}}
{{- $driver := include "duihua.background.autoscaling.driver" . -}}
{{- if eq $driver "redis-streams" -}}
{{- if hasKey $autoscaling "activationLagCount" -}}
{{- $autoscaling.activationLagCount -}}
{{- else if hasKey $autoscaling "activationTargetValue" -}}
{{- $autoscaling.activationTargetValue -}}
{{- else -}}
0
{{- end -}}
{{- else -}}
{{- if hasKey $autoscaling "activationTargetValue" -}}
{{- $autoscaling.activationTargetValue -}}
{{- else if hasKey $autoscaling "activationLagCount" -}}
{{- $autoscaling.activationLagCount -}}
{{- else -}}
0
{{- end -}}
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
{{- else -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- if eq (dig "responsesApiStore" "backgroundJobs" "enabled" true $gateway) false -}}
{{- else -}}
true
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.background.staleSeconds" -}}
{{- $store := get .Values "responses-api-store" | default dict -}}
{{- $storeCfg := $store.store | default dict -}}
{{- if hasKey $storeCfg "staleSeconds" -}}
{{- $storeCfg.staleSeconds -}}
{{- else -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $backgroundJobs := dig "responsesApiStore" "backgroundJobs" dict $gateway -}}
{{- if hasKey $backgroundJobs "staleSeconds" -}}
{{- get $backgroundJobs "staleSeconds" -}}
{{- else -}}
{{- .Values.backgroundWorker.staleSeconds | default 3600 -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "duihua.validate.config" -}}
{{- $gateway := index .Values "duihua-gateway" | default dict -}}
{{- $gatewayStore := $gateway.responsesApiStore | default dict -}}
{{- $gatewayEnv := $gateway.env | default dict -}}
{{- if .Values.inference.enabled -}}
{{- if not $gatewayEnv.modelUpstreams -}}
{{- fail (printf "inference.enabled=true requires duihua-gateway.env.modelUpstreams (e.g. %s)" (include "duihua.gateway.modelUpstreams" .)) -}}
{{- end -}}
{{- end -}}
{{- if eq (include "duihua.responsesApiStore.enabled" .) "true" -}}
{{- if not $gatewayStore.endpoint -}}
{{- if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- fail (printf "duihua-gateway.responsesApiStore.enabled=true requires duihua-gateway.responsesApiStore.endpoint (e.g. %s)" (include "duihua.responsesApiStoreEndpoint" .)) -}}
{{- else -}}
{{- fail "duihua-gateway.responsesApiStore.enabled=true requires responsesApiStoreService.enabled=true or duihua-gateway.responsesApiStore.endpoint" -}}
{{- end -}}
{{- end -}}
{{- $backgroundJobs := dig "responsesApiStore" "backgroundJobs" dict $gateway -}}
{{- if and (hasKey $backgroundJobs "consumerGroup") (ne (get $backgroundJobs "consumerGroup") .Values.backgroundWorker.consumerGroup) -}}
{{- fail (printf "duihua-gateway.responsesApiStore.backgroundJobs.consumerGroup (%v) must match backgroundWorker.consumerGroup (%v)" (get $backgroundJobs "consumerGroup") .Values.backgroundWorker.consumerGroup) -}}
{{- end -}}
{{- if and (not .Values.backgroundWorker.enabled) (dig "responsesApiStore" "backgroundJobs" "enabled" true $gateway) -}}
{{- fail "backgroundWorker.enabled=false requires duihua-gateway.responsesApiStore.backgroundJobs.enabled=false" -}}
{{- end -}}
{{- if eq (include "duihua.responsesApiStoreService.enabled" .) "true" -}}
{{- $store := get .Values "responses-api-store" | default dict -}}
{{- $configured := dig "store" "staleSeconds" 3600 $store | int -}}
{{- $backgroundJobs := dig "responsesApiStore" "backgroundJobs" dict $gateway -}}
{{- if hasKey $backgroundJobs "staleSeconds" -}}
{{- if ne (get $backgroundJobs "staleSeconds" | int) $configured -}}
{{- fail (printf "duihua-gateway.responsesApiStore.backgroundJobs.staleSeconds (%v) must match responses-api-store.store.staleSeconds (%v)" (get $backgroundJobs "staleSeconds") $configured) -}}
{{- end -}}
{{- end -}}
{{- if hasKey .Values.backgroundWorker "staleSeconds" -}}
{{- if ne (.Values.backgroundWorker.staleSeconds | int) $configured -}}
{{- fail (printf "backgroundWorker.staleSeconds (%v) must match responses-api-store.store.staleSeconds (%v); set responses-api-store.store.staleSeconds instead" .Values.backgroundWorker.staleSeconds $configured) -}}
{{- end -}}
{{- end -}}
{{- end -}}
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