# wasmCloud contrib

Community contributions of providers, components, and demos

## Components

- [moonbit/http-hello-world](./components/moonbit/http-hello-world/) is an example component that implements the `wasi-http/incoming-handler@0.2.0` interface and is built with [Moonbit](https://www.moonbitlang.com/)

## Deployments

Examples and scripts to help you deploy and operate wasmCloud on various platforms.

- [Akamai](./deploy/akamai/) for running wasmCloud on Akamai's LKE service
- [baremetal](./deploy/baremetal) with example systemd units and config files for 'baremetal' deployments
- [k8s](./deploy/k8s) for best practices, patterns, and examples for deploying wasmCloud on k8s
- [k8s kustomize / kind](./deploy/k8s/kustomize/) for a walkthrough of deploying wasmCloud with [kustomize](https://kubectl.docs.kubernetes.io/installation/kustomize/) on [kind](https://kind.sigs.k8s.io/docs/user/quick-start/)

## Providers

- [Couchbase](./providers/couchbase/) is a submodule of [couchbaselabs/wasmcloud-provider-couchbase](https://github.com/couchbaselabs/wasmcloud-provider-couchbase) brought here for wasmCloud signed releases of the provider

## Secrets

There are currently two implementations of [wasmCloud secrets backends](https://wasmcloud.com/docs/deployment/security/secrets#implementing-a-secrets-backend) available in this repository.

- [secrets-kubernetes](./secrets/secrets-kubernetes/) for using secrets stored in Kubernetes in wasmCloud applications
- [secrets-vault](./secrets/secrets-vault/) for using secrets stored in Vault in wasmCloud applications
