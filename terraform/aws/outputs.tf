output "vpc_id" {
  value = module.vpc.vpc_id
}

output "private_subnet_ids" {
  value = module.vpc.private_subnets
}

output "eks_cluster_name" {
  value = module.eks.cluster_name
}

output "eks_cluster_endpoint" {
  value = module.eks.cluster_endpoint
}

output "eks_oidc_issuer_url" {
  value = module.eks.cluster_oidc_issuer_url
}

output "rds_endpoint" {
  value     = module.rds.db_instance_endpoint
  sensitive = true
}

output "rds_password_secret" {
  description = "Postgres master password (read once; store in your secret manager)."
  value       = random_password.postgres.result
  sensitive   = true
}

output "pki_bundle_secret_arn" {
  value = aws_secretsmanager_secret.pki_bundle.arn
}

output "webhook_hmac_secret_arn" {
  value = aws_secretsmanager_secret.webhook_hmac.arn
}

output "bundles_bucket_name" {
  value = aws_s3_bucket.bundles.bucket
}

output "bundle_reader_policy_arn" {
  description = "Attach to the IAM role for IRSA used by sidecar / outbox-forwarder service accounts."
  value       = aws_iam_policy.bundle_reader.arn
}

output "kubeconfig_command" {
  description = "Run this after apply to populate ~/.kube/config."
  value       = "aws eks update-kubeconfig --region ${var.region} --name ${module.eks.cluster_name}"
}
