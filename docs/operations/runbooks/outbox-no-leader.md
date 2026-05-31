# Audit Outbox No Leader

Alert: `SpendGuardOutboxNoLeader`

## Detection

Prometheus fires when `spendguard_outbox_forwarder_is_leader` does not sum to exactly 1 for 5 minutes.

## Diagnosis

Check outbox-forwarder pod count, lease or advisory-lock logs, restart loops, and database connectivity. If the sum is 0, no pod owns forwarding. If the sum is greater than 1, investigate split-brain risk before scaling.

## Mitigation

For zero leaders, restore database connectivity and restart unhealthy forwarder pods. For multiple leaders, scale to one known-good replica, confirm only one leader metric remains, then scale back cautiously after logs show exclusive ownership.

## Rollback

Rollback the outbox-forwarder image or deployment config that changed leader election behavior. Restore the previous replica count only after one-leader state is stable for 10 minutes.

## Evidence

Record leader metric values per pod, deployment replica count, pod restart history, lease or lock logs, and pending outbox count before and after recovery.

## Safety

Do not run manual concurrent forwarder loops outside the deployment controller. Keep canonical-ingest strict signature verification active.
