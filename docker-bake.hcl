# SPDX-License-Identifier: Apache-2.0
# Docker Bake configuration for QuackGIS container builds.
#
# Usage:
#   docker bake sedonadb-builder   # build only the Rust compilation stage
#   docker bake runtime            # build the full runtime image
#   docker bake --push             # build and push all targets
#
# The sedonadb-builder target is cached separately because it changes
# infrequently (only on Rust source changes) while the runtime layer
# changes with init script / config updates.

variable "IMAGE" {
  default = "quackgis"
}

variable "TAG" {
  default = "dev"
}

variable "REGISTRY" {
  default = ""
}

group "default" {
  targets = ["runtime"]
}

target "sedonadb-builder" {
  dockerfile = "container/Dockerfile"
  target = "sedonadb-builder"
  cache-from = [
    "type=local,src=.cache/sedonadb-builder"
  ]
  cache-to = [
    "type=local,dest=.cache/sedonadb-builder,mode=max"
  ]
}

target "runtime" {
  dockerfile = "container/Dockerfile"
  target = "runtime"
  tags = [
    "${REGISTRY != "" ? "${REGISTRY}/" : ""}${IMAGE}:${TAG}",
    "${REGISTRY != "" ? "${REGISTRY}/" : ""}${IMAGE}:latest"
  ]
  cache-from = [
    "type=local,src=.cache/sedonadb-builder",
    "type=local,src=.cache/runtime"
  ]
  cache-to = [
    "type=local,dest=.cache/runtime,mode=max"
  ]
  platforms = ["linux/amd64"]
  labels = {
    "org.opencontainers.image.title" = "QuackGIS"
    "org.opencontainers.image.description" = "PostGIS-compatible spatial database facade"
    "org.opencontainers.image.source" = "https://github.com/adonm/quackgis"
    "org.opencontainers.image.licenses" = "Apache-2.0"
  }
}
