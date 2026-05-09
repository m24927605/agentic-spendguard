# Terraform deployment (AWS)

```bash
cd terraform/aws
cp example.tfvars terraform.tfvars
# edit terraform.tfvars

terraform init
terraform plan
terraform apply

$(terraform output -raw kubeconfig_command)

# Then bootstrap chart Secrets from Terraform outputs and
helm install spendguard ./charts/spendguard ...
```

Provisions VPC + EKS + RDS Postgres + Secrets Manager + S3 bundle
bucket + IRSA policy.

Cost estimates and POC limits in
[terraform/aws/README.md](https://github.com/m24927605/agentic-flow-cost-evaluation/blob/main/terraform/aws/README.md).
