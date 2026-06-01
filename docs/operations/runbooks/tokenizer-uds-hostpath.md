# Tokenizer UDS HostPath Runbook

## Purpose

POST_GA_03 / #105: production tokenizer UDS mode requires a pre-created node
directory. Do not rely on `DirectoryOrCreate`; kubelet creates root-owned paths
and the tokenizer runs as UID/GID 65532 with a read-only root filesystem.

## Required Values

```yaml
tokenizer:
  udsHostPath: /var/run/spendguard-tokenizer
  udsHostPathType: Directory
```

The production validation template rejects `tokenizer.udsHostPathType` values
other than `Directory`.

## Pre-Provision

Run an OS-level node bootstrap step, image bake step, or privileged cluster
bootstrap DaemonSet before installing SpendGuard:

```yaml
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: spendguard-tokenizer-uds-prep
  namespace: kube-system
spec:
  selector:
    matchLabels:
      app: spendguard-tokenizer-uds-prep
  template:
    metadata:
      labels:
        app: spendguard-tokenizer-uds-prep
    spec:
      tolerations:
        - operator: Exists
      containers:
        - name: prep
          image: busybox:1.36
          command: ["sh", "-c", "mkdir -p /host/var/run/spendguard-tokenizer && chown 65532:65532 /host/var/run/spendguard-tokenizer && chmod 0770 /host/var/run/spendguard-tokenizer && sleep 3600"]
          securityContext:
            runAsUser: 0
            allowPrivilegeEscalation: false
          volumeMounts:
            - name: host-var-run
              mountPath: /host/var/run
      volumes:
        - name: host-var-run
          hostPath:
            path: /var/run
            type: Directory
```

Remove the DaemonSet after every node reports the directory with owner
`65532:65532` and mode `0770`, or keep an equivalent node bootstrap mechanism
in place for node autoscaling.

## Verify

```bash
kubectl get pods -l app.kubernetes.io/component=tokenizer -o wide
kubectl exec deploy/spendguard-tokenizer -- stat -c '%u:%g %a %n' /var/run/spendguard
kubectl exec deploy/spendguard-tokenizer -- test -S /var/run/spendguard/tokenizer.sock
```

Expected:

- `stat` shows `65532:65532 770` or stricter group-writable equivalent.
- `tokenizer.sock` exists after the tokenizer pod is ready.
- `helm template charts/spendguard --set chart.profile=production --set tokenizer.udsHostPath=/var/run/spendguard-tokenizer --set tokenizer.udsHostPathType=Directory ...` renders cleanly.

## Rollback

Unset `tokenizer.udsHostPath` and configure `tokenizer.mtlsSecretName` to expose
the tokenizer through the mTLS ClusterIP Service. Do not switch to plaintext TCP
in production.
