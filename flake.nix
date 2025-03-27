{
  description = "Route53 Dynamic DNS Client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
  };

  outputs = {self, nixpkgs}: {
    defaultPackage.x86_64-linux =
      with import nixpkgs { system = "x86_64-linux"; };

      stdenv.mkDerivation rec {
        name = "route53_dd-${version}";

        version = "0.1.0-d78329e7";

        # https://nixos.wiki/wiki/Packaging/Binaries
        executable = pkgs.fetchurl {
          url = "https://github.com/JordanSekky/route53_dd/releases/download/${version}/route53_dd";
          sha256 = "sha256-k2B2z+CDOf3QnD6f1RT0NUdXeQ4ksuaqYmMcOnaYa4c=";
        };

        phases = [ "installPhase" ]; # Removes all phases except installPhase

        installPhase = ''
        mkdir -p $out/bin
        install -m755 -D ${executable} $out/bin/route53_dd
        '';

        meta = with lib; {
          homepage = "https://github.com/JordanSekky/route53_dd";
          description = "Single-executable Route53 Dynamic DNS Client";
          platforms = platforms.linux;
        };
      };
  };
}
