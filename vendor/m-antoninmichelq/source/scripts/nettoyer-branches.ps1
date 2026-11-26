# Cleans the branch list, on the fork and in the PC's clone, without losing anything.
#
# Run from C:\Users\4nto\Desktop\VIctory, with no VIctory running:
#   powershell -ExecutionPolicy Bypass -File scripts\nettoyer-branches.ps1           (shows what it would do)
#   powershell -ExecutionPolicy Bypass -File scripts\nettoyer-branches.ps1 -Apply    (does it)
#
# 1. Every branch tip that will be removed (fork or local) and is not yet saved is added as a parent of
#    the fork branch archive/branches (one commit whose parents are all the old tips + ARCHIVE.md list),
#    then pushed. Restore: git branch <name> <sha> (sha from ARCHIVE.md / this script's output).
# 2. Fork branches not in $Keep are deleted.
# 3. Clean extra worktrees are removed (a worktree with changes is left alone), then local branches
#    not in $Keep are deleted (never the checked-out one).
param([switch]$Apply)

# Text piped into git must not start with a UTF-8 BOM (git mktree then fails and the archive commit is empty).
$OutputEncoding = New-Object Text.UTF8Encoding $false

$Keep = @(
  'main', 'docs/handoff', 'archive/branches',
  'jouer/peau3', 'jouer/peau2', 'jouer/peau3-overlay',
  'essai/groupe', 'essai/groupe-base', 'essai/groupe-overlay',
  'perf/lean-math', 'perf/clamp-ledger', 'perf/tiles-probe',
  'fix/loading-sharp', 'fix/evict-grey', 'fix/wait-grey', 'fix/partly-clamp', 'port/fx-d510ee4', 'scratch/smem'
)
$ForkUrl = 'https://github.com/AntoninMichelq/VIctory-forkAMD'

if (Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -like 'victory*' }) {
  Write-Host 'VIctory tourne : je ne touche a rien. Relance quand le jeu est ferme.'
  exit 1
}

function GitOk { & git.exe @args; if ($LASTEXITCODE -ne 0) { throw "git $args a echoue" } }

if (-not ((git remote) -contains 'fork')) { GitOk remote add fork $ForkUrl }
GitOk fetch fork --prune

$current = (git branch --show-current)
$forkBranches = git ls-remote --heads fork | ForEach-Object { ($_ -split "`t")[1] -replace '^refs/heads/', '' }
$forkDel = @($forkBranches | Where-Object { $Keep -notcontains $_ })
$localBranches = git for-each-ref --format='%(refname:short)' refs/heads
$localDel = @($localBranches | Where-Object { ($Keep -notcontains $_) -and ($_ -ne $current) })

# Tips to save: fork + local branches going away that archive/branches does not already contain.
$toSave = [ordered]@{}
foreach ($b in $forkDel) { $toSave["$b"] = (git rev-parse "fork/$b") }
foreach ($b in $localDel) { $toSave["local/$b"] = (git rev-parse "refs/heads/$b") }
$archive = (git rev-parse fork/archive/branches)
$new = [ordered]@{}
foreach ($k in $toSave.Keys) {
  git merge-base --is-ancestor $toSave[$k] $archive
  if ($LASTEXITCODE -ne 0) { $new[$k] = $toSave[$k] }
}

Write-Host "Branches du fork a supprimer : $($forkDel.Count)"
Write-Host "Branches locales a supprimer : $($localDel.Count)"
Write-Host "Pointes pas encore archivees : $($new.Count)"
$new.Keys | ForEach-Object { Write-Host "  $_ $($new[$_])" }
if (-not $Apply) { Write-Host 'Mode essai : rien n''a ete change. Ajoute -Apply pour le faire.'; exit 0 }

if ($new.Count -gt 0) {
  $list = (git show "${archive}:ARCHIVE.md") -join "`n"
  $date = Get-Date -Format 'yyyy-MM-dd HH:mm'
  $list += "`n`n## Added $date (from the PC)`n`n| branch | commit |`n|---|---|`n"
  $parents = @('-p', $archive)
  $seen = @{ $archive = 1 }
  foreach ($k in $new.Keys) {
    $list += "| $k | $($new[$k]) |`n"
    if (-not $seen.ContainsKey($new[$k])) { $parents += @('-p', $new[$k]); $seen[$new[$k]] = 1 }
  }
  $tmp = [IO.Path]::GetTempFileName()
  [IO.File]::WriteAllText($tmp, $list, (New-Object Text.UTF8Encoding $false))
  $blob = (git hash-object -w $tmp)
  $tree = ("100644 blob $blob`tARCHIVE.md" | git mktree)
  if ($tree -notmatch '^[0-9a-f]{40}$') { throw "git mktree a echoue ('$tree') : rien n'est pousse" }
  $commit = (git commit-tree $tree @parents -m "Archive: branches removed on $date")
  if ($commit -notmatch '^[0-9a-f]{40}$') { throw "git commit-tree a echoue ('$commit') : rien n'est pousse" }
  git merge-base --is-ancestor $archive $commit
  if ($LASTEXITCODE -ne 0) { throw 'le nouveau commit d''archive ne contient pas l''ancien : rien n''est pousse' }
  GitOk push fork "${commit}:refs/heads/archive/branches"
  Remove-Item $tmp
}

for ($i = 0; $i -lt $forkDel.Count; $i += 40) {
  $chunk = $forkDel[$i..([Math]::Min($i + 39, $forkDel.Count - 1))]
  GitOk push fork --delete @chunk
}

# Worktrees other than this folder: remove only clean ones on a branch that goes away.
$here = (Resolve-Path .).Path.Replace('\', '/').ToLower()
$wt = $null
foreach ($line in (git worktree list --porcelain)) {
  if ($line -like 'worktree *') { $wt = $line.Substring(9) }
  elseif ($line -like 'branch *' -and $wt) {
    $b = $line.Substring(7) -replace '^refs/heads/', ''
    if ($wt.Replace('\', '/').ToLower() -ne $here -and $Keep -notcontains $b) {
      git worktree remove $wt
      if ($LASTEXITCODE -ne 0) { Write-Host "Worktree garde (modifications en cours) : $wt" }
    }
  }
}
git worktree prune

$still = git worktree list --porcelain | Where-Object { $_ -like 'branch *' } | ForEach-Object { $_.Substring(7) -replace '^refs/heads/', '' }
foreach ($b in $localDel) {
  if ($still -contains $b) { Write-Host "Branche gardee (utilisee par un worktree) : $b"; continue }
  git branch -D $b | Out-Null
}
Write-Host 'Fini. Branches restantes :'
git branch
