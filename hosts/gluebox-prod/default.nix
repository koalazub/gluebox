{ self, pkgs, lib, config, ... }:
let
  glueboxPkg = self.packages.x86_64-linux.gluebox;
  valkeyBloomPkg = self.packages.x86_64-linux.valkey-bloom;
in
{
  imports = [ ./hardware-configuration.nix ];

  boot.loader.grub.enable = true;
  boot.loader.grub.device = "/dev/vda";

  networking.hostName = "gluebox-prod";
  time.timeZone = "UTC";

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.settings.trusted-users = [ "root" ];

  users.users.root.openssh.authorizedKeys.keys = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMkn5pAtft3oahcYHzXtgURz6g+cUZbS9euMgAHarF+8 koalazub@KoalaBook.local"
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHRZighEl9bRZUwPGkIefAFmi1y8L6tSSkv8+zUXMVp7 koalazub@KoalaBook.local-2026"
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICoZ6NjPAXJJCt/Doqlg1rlrrkIIdCYMcg90CHbK2wfl gluebox-deploy"
    "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBExIAAQ51kcUa02F73izB9v8Hlp7f4RUSrGQWgQgtp35daM9qfQtBDzjojucOwwjdREhtMfewVeCI3eGxffFPys= gluebox-nixos@secretive.KoalaBook.local"
  ];

  services.openssh = {
    enable = true;
    settings.PermitRootLogin = "prohibit-password";
    settings.PasswordAuthentication = false;
  };

  services.tailscale = {
    enable = true;
    useRoutingFeatures = "server";
  };

  systemd.services.tailscale-funnel = {
    description = "Tailscale Funnel for gluebox webhooks";
    after = [ "tailscaled.service" "gluebox.service" ];
    wants = [ "tailscaled.service" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    path = [ pkgs.tailscale ];
    script = ''
      sleep 5
      tailscale funnel --bg http://127.0.0.1:8990
    '';
  };

  services.redis = {
    package = pkgs.valkey;
    servers."" = {
      enable = true;
      bind = "127.0.0.1";
      port = 6379;
      settings = {
        maxmemory = "128mb";
        loadmodule = [ "${valkeyBloomPkg}/lib/libvalkey_bloom.so" ];
        enable-module-command = "local";
      };
    };
  };

  systemd.services.gluebox = {
    description = "Gluebox webhook server";
    after = [ "network-online.target" "redis.service" ];
    wants = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "${glueboxPkg}/bin/gluebox";
      Restart = "always";
      RestartSec = 30;
      StartLimitIntervalSec = 0;
      StateDirectory = "gluebox";
    };
    path = [ pkgs.typst ];
    environment = {
      GLUEBOX_CONFIG = "/etc/gluebox/gluebox.toml";
      RUST_LOG = "gluebox=info";
    };
    preStart = ''
      mkdir -p /var/lib/gluebox/og-images
    '';
  };

  networking.firewall = {
    enable = true;
    allowedTCPPorts = [ 22 ];
    trustedInterfaces = [ "tailscale0" ];
  };

  environment.etc."gluebox/og-card.typ".source = ../../assets/og-card.typ;
  environment.etc."gluebox/story-card.typ".source = ../../assets/story-card.typ;

  fonts.packages = [ pkgs.jetbrains-mono ];

  environment.systemPackages = with pkgs; [
    curl
    htop
    tailscale
    typst
  ];

  swapDevices = [{
    device = "/var/swapfile";
    size = 2048;
  }];

  system.stateVersion = "25.05";
}
