$inputData = "12`n+`n34`nq`n"

function Test-Speed {
    param(
        [string]$name,
        [scriptblock]$command
    )
    $total = 0
    for ($i = 0; $i -lt 10; $i++) {
        $time = Measure-Command -Expression {
            $inputData | & $command > $null 2>&1
        }
        $total += $time.TotalMilliseconds
    }
    $avg = $total / 10
    Write-Host ("{0,-20} | {1:N2} ms" -f $name, $avg)
}

Write-Host "Average execution time (10 runs cold-start, end-to-end IO piped):"
Write-Host "------------------------------------------------------------------"
Test-Speed "C Native (.exe)" { .\examples\calculator.exe }
Test-Speed "FASM (Compile+Run)" { .\target\release\fasm.exe run .\examples\calculator.fasm }
Test-Speed "FASM (Bytecode)" { .\target\release\fasm.exe run .\examples\calculator.fasmc }
Test-Speed "Node.js" { node .\examples\calculator.js }
Test-Speed "Python" { python .\examples\calculator.py }
