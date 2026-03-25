param(
    [int]$Length = 48
)

$alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"
$bytes = New-Object byte[] $Length
[System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
$chars = for ($i = 0; $i -lt $Length; $i++) {
    $alphabet[$bytes[$i] % $alphabet.Length]
}
$token = -join $chars

$sha = [System.Security.Cryptography.SHA256]::Create()
$hashBytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($token))
$hashHex = [System.BitConverter]::ToString($hashBytes).Replace("-", "").ToLowerInvariant()

Write-Host "Generated API token:"
Write-Host $token
Write-Host ""
Write-Host "SHA256 hash (store in VIRALCLIP_API_TOKEN_SHA256):"
Write-Host $hashHex
Write-Host ""
Write-Host "Recommended env setup:"
Write-Host '$env:VIRALCLIP_API_TOKEN_SHA256="<hash-from-above>"'
