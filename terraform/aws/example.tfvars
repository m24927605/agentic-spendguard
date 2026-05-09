# Example tfvars — copy to terraform.tfvars and customize.
# Save outputs (especially rds_password_secret) somewhere safe;
# `random_password` regenerates on plan if you re-run from empty state.

region              = "us-east-1"
name_prefix         = "spendguard-dev"

availability_zones  = ["us-east-1a", "us-east-1b"]
vpc_cidr            = "10.40.0.0/16"

# Cost-vs-HA tradeoffs (cheap-side defaults)
multi_az_nat                  = false   # 1 NAT, ~$32/mo each AZ
multi_az_rds                  = false   # set true for production
eks_public_endpoint           = false
node_instance_type            = "t3.medium"
node_group_desired            = 2
rds_instance_class            = "db.t4g.small"
rds_allocated_storage_gib     = 20
rds_backup_retention_days     = 7
secrets_recovery_window_days  = 0       # 0 = no recovery (dev)

tags = {
  Environment = "dev"
  Owner       = "platform-team"
}
