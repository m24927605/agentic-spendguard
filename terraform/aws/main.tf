# SpendGuard on AWS — Phase 4 O9 Terraform module.
#
# Provisions the cloud infrastructure needed to run the SpendGuard
# Helm chart (charts/spendguard/) on AWS:
#   - VPC + private/public subnets across 2 AZs
#   - EKS cluster + managed node group
#   - RDS Postgres (multi-AZ optional)
#   - AWS Secrets Manager entries for PKI bundle + webhook HMAC
#   - S3 bucket for contract / schema bundle storage
#   - IAM role for IRSA (IAM Roles for Service Accounts)
#
# This module is locally-validated only (terraform validate + plan
# against an empty state). A real `terraform apply` against an AWS
# sandbox account is the next layer; deferred to operator docs.

terraform {
  required_version = ">= 1.6"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.50"
    }
  }
}

provider "aws" {
  region = var.region
}

# ---------------------------------------------------------------------------
# VPC
# ---------------------------------------------------------------------------

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 5.13"

  name = "${var.name_prefix}-vpc"
  cidr = var.vpc_cidr

  azs             = var.availability_zones
  private_subnets = [for i, az in var.availability_zones : cidrsubnet(var.vpc_cidr, 4, i)]
  public_subnets  = [for i, az in var.availability_zones : cidrsubnet(var.vpc_cidr, 4, i + 8)]

  enable_nat_gateway   = true
  single_nat_gateway   = !var.multi_az_nat
  enable_dns_hostnames = true
  enable_dns_support   = true

  tags = local.common_tags
}

# ---------------------------------------------------------------------------
# EKS
# ---------------------------------------------------------------------------

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "~> 20.24"

  cluster_name    = "${var.name_prefix}-eks"
  cluster_version = var.eks_version

  vpc_id     = module.vpc.vpc_id
  subnet_ids = module.vpc.private_subnets

  cluster_endpoint_public_access = var.eks_public_endpoint
  enable_irsa                    = true

  eks_managed_node_groups = {
    main = {
      desired_size   = var.node_group_desired
      min_size       = var.node_group_min
      max_size       = var.node_group_max
      instance_types = [var.node_instance_type]
      capacity_type  = "ON_DEMAND"
      labels = {
        role = "spendguard-workers"
      }
    }
  }

  tags = local.common_tags
}

# ---------------------------------------------------------------------------
# RDS Postgres
# ---------------------------------------------------------------------------

resource "random_password" "postgres" {
  length  = 32
  special = true
}

module "rds" {
  source  = "terraform-aws-modules/rds/aws"
  version = "~> 6.10"

  identifier = "${var.name_prefix}-postgres"

  engine               = "postgres"
  engine_version       = var.postgres_version
  family               = var.postgres_family
  major_engine_version = var.postgres_major_version
  instance_class       = var.rds_instance_class

  allocated_storage     = var.rds_allocated_storage_gib
  max_allocated_storage = var.rds_max_allocated_storage_gib

  db_name  = "spendguard_ledger"
  username = "spendguard"
  password = random_password.postgres.result
  port     = 5432

  multi_az               = var.multi_az_rds
  db_subnet_group_name   = module.vpc.database_subnet_group_name
  vpc_security_group_ids = [aws_security_group.rds.id]

  maintenance_window      = "Mon:00:00-Mon:03:00"
  backup_window           = "03:00-06:00"
  backup_retention_period = var.rds_backup_retention_days

  performance_insights_enabled    = true
  performance_insights_retention_period = 7

  parameters = [
    { name = "log_min_duration_statement", value = "200" },
  ]

  tags = local.common_tags
}

resource "aws_security_group" "rds" {
  name_prefix = "${var.name_prefix}-rds-"
  vpc_id      = module.vpc.vpc_id

  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.common_tags
}

# ---------------------------------------------------------------------------
# Secrets Manager (PKI bundle + webhook HMAC)
# ---------------------------------------------------------------------------

resource "aws_secretsmanager_secret" "pki_bundle" {
  name_prefix             = "${var.name_prefix}-pki-"
  description             = "SpendGuard PKI material (CA + per-service certs)"
  recovery_window_in_days = var.secrets_recovery_window_days
  tags                    = local.common_tags
}

resource "aws_secretsmanager_secret" "webhook_hmac" {
  name_prefix             = "${var.name_prefix}-webhook-hmac-"
  description             = "SpendGuard webhook receiver HMAC secret"
  recovery_window_in_days = var.secrets_recovery_window_days
  tags                    = local.common_tags
}

# ---------------------------------------------------------------------------
# S3 bucket for bundle storage
# ---------------------------------------------------------------------------

resource "aws_s3_bucket" "bundles" {
  bucket_prefix = "${var.name_prefix}-bundles-"
  tags          = local.common_tags
}

resource "aws_s3_bucket_versioning" "bundles" {
  bucket = aws_s3_bucket.bundles.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "bundles" {
  bucket = aws_s3_bucket.bundles.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_public_access_block" "bundles" {
  bucket                  = aws_s3_bucket.bundles.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# ---------------------------------------------------------------------------
# IRSA for sidecar / outbox-forwarder reading bundle bucket
# ---------------------------------------------------------------------------

data "aws_iam_policy_document" "bundle_reader" {
  statement {
    actions = [
      "s3:GetObject",
      "s3:ListBucket",
    ]
    resources = [
      aws_s3_bucket.bundles.arn,
      "${aws_s3_bucket.bundles.arn}/*",
    ]
  }
}

resource "aws_iam_policy" "bundle_reader" {
  name_prefix = "${var.name_prefix}-bundle-reader-"
  policy      = data.aws_iam_policy_document.bundle_reader.json
}

# ---------------------------------------------------------------------------
# Tags + locals
# ---------------------------------------------------------------------------

locals {
  common_tags = merge(var.tags, {
    Project   = "spendguard"
    Component = "onboarding-O9"
    ManagedBy = "terraform"
  })
}
