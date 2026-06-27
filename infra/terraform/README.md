# LogMux Terraform Modules

This directory contains Terraform/OpenTofu modules for provisioning multi-AZ VPC infrastructure.

## Structure

```
infra/terraform/
├── main.tf        # VPC, subnets, route tables, security groups
├── variables.tf   # Input variables
├── outputs.tf     # Export values for k8s modules
└── providers.tf   # Provider requirements
```

## Usage

```hcl
module "logmux_vpc" {
  source = "./infra/terraform"

  region  = "us-east-1"
  azs     = ["us-east-1a", "us-east-1b", "us-east-1c"]
  cidr    = "10.0.0.0/16"
}
```