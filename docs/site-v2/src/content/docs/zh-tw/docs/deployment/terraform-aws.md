---
title: "Terraform 部署（AWS）"
---

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

會幫你開好 VPC + EKS + RDS Postgres + Secrets Manager + S3 bundle bucket + IRSA policy。

成本估算跟 POC 的限制都寫在
[terraform/aws/README.md](https://github.com/m24927605/agentic-spendguard/blob/main/terraform/aws/README.md)。
