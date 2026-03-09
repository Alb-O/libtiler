{ lib, ... }:

{
  instructions.instructions = lib.mkAfter [
    (
      if builtins.pathExists ./AGENTS.md
      then builtins.readFile ./AGENTS.md
      else ""
    )
  ];

  enterShell = ''
    echo "Run: run-tests"
    echo "Run: lint"
    echo "Run: check-targets"
  '';
}
