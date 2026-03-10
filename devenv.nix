{ lib, ... }:

{
  composer.ownInstructions =
    let
      currentProject = builtins.baseNameOf (toString ./.);
    in
    lib.optionalAttrs (builtins.pathExists ./AGENTS.md) {
      "${currentProject}" = [ (builtins.readFile ./AGENTS.md) ];
    };

  enterShell = ''
    echo "Run: run-tests"
    echo "Run: lint"
    echo "Run: check-targets"
  '';
}
