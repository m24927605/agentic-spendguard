variable "region" {
  description = "AWS region (e.g. us-east-1)."
  type        = string
}

variable "name_prefix" {
  description = "Prefix for all created resources (e.g. spendguard-prod)."
  type        = string
  default     = "spendguard"
}

variable "vpc_cidr" {
  description = "CIDR block for the VPC."
  type        = string
  default     = "10.40.0.0/16"
}

variable "availability_zones" {
  description = "AZs to span (typically 2 for cost, 3 for HA)."
  type        = list(string)
}

variable "multi_az_nat" {
  description = "If true, provision a NAT gateway per AZ (HA but expensive)."
  type        = bool
  default     = false
}

variable "multi_az_rds" {
  description = "Enable RDS multi-AZ deployment."
  type        = bool
  default     = false
}

variable "eks_version" {
  description = "Kubernetes version for the EKS cluster."
  type        = string
  default     = "1.30"
}

variable "eks_public_endpoint" {
  description = "Expose the EKS API endpoint publicly (still IAM-gated)."
  type        = bool
  default     = false
}

variable "node_instance_type" {
  description = "EC2 instance type for EKS managed node group."
  type        = string
  default     = "t3.large"
}

variable "node_group_desired" {
  description = "Desired count for the managed node group."
  type        = number
  default     = 2
}

variable "node_group_min" {
  description = "Minimum count for the managed node group."
  type        = number
  default     = 1
}

variable "node_group_max" {
  description = "Maximum count for the managed node group."
  type        = number
  default     = 5
}

variable "postgres_version" {
  description = "Postgres engine version."
  type        = string
  default     = "16.4"
}

variable "postgres_major_version" {
  description = "Postgres major version (matches family)."
  type        = string
  default     = "16"
}

variable "postgres_family" {
  description = "RDS parameter group family."
  type        = string
  default     = "postgres16"
}

variable "rds_instance_class" {
  description = "RDS instance class. db.t4g.medium is ~$50/mo; smaller for dev."
  type        = string
  default     = "db.t4g.medium"
}

variable "rds_allocated_storage_gib" {
  description = "Initial RDS allocated storage in GiB."
  type        = number
  default     = 50
}

variable "rds_max_allocated_storage_gib" {
  description = "Max storage RDS can autoscale to."
  type        = number
  default     = 500
}

variable "rds_backup_retention_days" {
  description = "RDS automated backup retention in days."
  type        = number
  default     = 14
}

variable "secrets_recovery_window_days" {
  description = "Secrets Manager recovery window. 0 = immediate delete."
  type        = number
  default     = 7
}

variable "tags" {
  description = "Custom tags applied to all resources."
  type        = map(string)
  default     = {}
}
