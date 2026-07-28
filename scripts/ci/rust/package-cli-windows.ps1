#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory=$true)]
    [string]$Target
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Host "=== Packaging CLI binary for $Target ==="

cd target/$Target/release
Compress-Archive -Path xberg.exe -DestinationPath ../../../xberg-cli-$Target.zip

Write-Host "Packaging complete: xberg-cli-$Target.zip"
