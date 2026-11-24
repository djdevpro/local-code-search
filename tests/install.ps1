# Vérifie sous Windows qu'un binaire téléchargé s'installe seul : lcs.exe dans ~\.local\bin, et ce
# dossier ajouté au PATH de l'utilisateur (registre) sans rien perdre de son contenu.
# Écrit dans le vrai profil : à lancer sur une machine d'intégration, pas sur un poste.
# Usage : tests/install.ps1 <binaire>
param([Parameter(Mandatory)][string]$Binary)
$ErrorActionPreference = 'Stop'

$dir = Join-Path $env:USERPROFILE '.local\bin'
$work = Join-Path ([IO.Path]::GetTempPath()) "lcs-install-$PID"
New-Item -ItemType Directory $work | Out-Null
$download = Join-Path $work 'lcs-windows-x64.exe'
Copy-Item $Binary $download

# La valeur brute du registre, variables non développées.
function UserPath { [string](Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames') }
function Check($ok, $message) { if (-not $ok) { throw $message } }

$before = UserPath
$expected = & $Binary --version
Check ($LASTEXITCODE -eq 0) 'le binaire ne démarre pas'

Write-Host '== première commande du binaire téléchargé'
& $download --version
Check ($LASTEXITCODE -eq 0) 'le binaire téléchargé a échoué'
Check (Test-Path (Join-Path $dir 'lcs.exe')) "lcs.exe absent de $dir"
$after = UserPath
Check (($after -split ';') -contains $dir) "PATH utilisateur sans $dir : $after"
Check ($after.StartsWith($before, [StringComparison]::Ordinal)) "PATH utilisateur abîmé : « $before » devenu « $after »"

Write-Host '== un nouveau terminal trouve lcs'
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
$got = lcs --version
Check ($got -eq $expected) "lcs --version : « $got », attendu « $expected »"

Write-Host '== relancé, il ne change plus rien'
lcs install
Check ($LASTEXITCODE -eq 0) 'lcs install a échoué'
& $download install
Check ($LASTEXITCODE -eq 0) 'install depuis le binaire téléchargé a échoué'
Check ((UserPath) -ceq $after) 'PATH utilisateur modifié une seconde fois'

Write-Host '== sans commande (double-clic)'
'' | & $download
Check ($LASTEXITCODE -eq 0) 'le lancement sans commande a échoué'
Check ((UserPath) -ceq $after) 'PATH utilisateur modifié par le lancement sans commande'

Remove-Item -Recurse -Force $work
Write-Host 'Installation vérifiée.'
