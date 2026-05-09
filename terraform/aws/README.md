# SpendGuard on AWS — Terraform module

Provisions the AWS infrastructure SpendGuard's Helm chart needs.

## Resources

| Resource | Purpose |
|---|---|
| VPC + 2 AZs (private/public subnets) | Network |
| NAT gateway(s) | Outbound from private subnets |
| EKS cluster + managed node group | Compute |
| RDS Postgres | Ledger + canonical DBs |
| Secrets Manager entries | PKI bundle + webhook HMAC secrets |
| S3 bucket | Contract / schema bundle storage |
| IAM policy for IRSA | Sidecar service-account read access to S3 |

## Usage

```bash
cd terraform/aws
cp example.tfvars terraform.tfvars
# edit terraform.tfvars

terraform init
terraform validate
terraform plan
terraform apply

# After apply:
$(terraform output -raw kubeconfig_command)

# Then bootstrap the chart secrets from Terraform outputs and
# `helm install spendguard ./charts/spendguard ...` per
# charts/spendguard/README.md.
```

## Cost guardrails (defaults)

- `multi_az_nat = false` → 1 NAT (~$32/mo) instead of N
- `multi_az_rds = false` → single-AZ RDS
- `db.t4g.small` → ~$25/mo
- `t3.medium` × 2 EKS nodes → ~$60/mo
- Total dev environment: ~$150–200/mo + data transfer

Set `multi_az_*` and larger instance classes for production.

## Local validation

```bash
terraform fmt -check
terraform init -backend=false
terraform validate
```

## POC limits

- **No remote state backend configured.** Add an S3 backend (with
  DynamoDB lock) before any team usage:
  ```hcl
  terraform {
    backend "s3" {
      bucket         = "your-tfstate-bucket"
      key            = "spendguard/terraform.tfstate"
      region         = "us-east-1"
      dynamodb_table = "your-tf-lock-table"
    }
  }
  ```
- **No real apply tested.** This module passes `terraform validate`;
  end-to-end apply against an AWS sandbox is the next layer of
  validation (deferred to O10 docs).
- **Single-region.** Multi-region failover (per Stage 2 spec
  "cross_region_failover") is GA; not in this slice.
- **Postgres password lives in random_password.** That's a Terraform
  state thing — anyone with state read can recover it. For
  production, switch to AWS Secrets Manager-managed RDS password.
