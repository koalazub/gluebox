{ self, pkgs, lib, config, ... }:
let
  glueboxPkg = self.packages.x86_64-linux.gluebox;
  anySyncBundlePkg = self.packages.x86_64-linux.any-sync-bundle;
  valkeyBloomPkg = self.packages.x86_64-linux.valkey-bloom;
in
{
  imports = [ ./hardware-configuration.nix ];

  nixpkgs.config.allowUnfreePredicate = pkg: builtins.elem (lib.getName pkg) [
    "mongodb"
  ];

  boot.loader.grub.enable = true;
  boot.loader.grub.device = "/dev/vda";

  networking.hostName = "gluebox-prod";
  time.timeZone = "UTC";

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.settings.trusted-users = [ "root" ];

  users.users.root.openssh.authorizedKeys.keys = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMkn5pAtft3oahcYHzXtgURz6g+cUZbS9euMgAHarF+8 koalazub@KoalaBook.local"
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICoZ6NjPAXJJCt/Doqlg1rlrrkIIdCYMcg90CHbK2wfl gluebox-deploy"
  ];

  services.openssh = {
    enable = true;
    settings.PermitRootLogin = "prohibit-password";
    settings.PasswordAuthentication = false;
  };

  services.tailscale.enable = true;

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

  services.mongodb = {
    enable = true;
    bind_ip = "127.0.0.1";
    extraConfig = ''
      storage:
        wiredTiger:
          engineConfig:
            cacheSizeGB: 0.3
      replication:
        replSetName: rs0
    '';
  };

  systemd.services.mongodb-rs-init = {
    description = "Initialize MongoDB replica set";
    after = [ "mongodb.service" ];
    requires = [ "mongodb.service" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    path = [ pkgs.mongosh ];
    script = ''
      sleep 3
      mongosh --quiet --eval '
        try { rs.status() }
        catch(e) { rs.initiate({_id:"rs0", members:[{_id:0, host:"127.0.0.1:27017"}]}) }
      '
    '';
  };

  systemd.services.any-sync-bundle = {
    description = "Anytype any-sync-bundle server";
    after = [ "network-online.target" "mongodb.service" "mongodb-rs-init.service" "redis.service" ];
    wants = [ "network-online.target" ];
    requires = [ "mongodb.service" "redis.service" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = lib.concatStringsSep " " [
        "${anySyncBundlePkg}/bin/any-sync-bundle"
        "start-bundle"
        "--initial-external-addrs" "gluebox-prod.<tailnet>.ts.net"
        "--initial-mongo-uri" "mongodb://127.0.0.1:27017/"
        "--initial-redis-uri" "redis://127.0.0.1:6379/"
        "--initial-storage" "/var/lib/any-sync-bundle/storage"
        "--bundle-config" "/var/lib/any-sync-bundle/bundle-config.yml"
        "--client-config" "/var/lib/any-sync-bundle/client-config.yml"
      ];
      Restart = "on-failure";
      RestartSec = 5;
      WorkingDirectory = "/var/lib/any-sync-bundle";
      StateDirectory = "any-sync-bundle";
    };
  };

  systemd.services.gluebox = {
    description = "Gluebox webhook server";
    after = [ "network-online.target" "mongodb.service" "redis.service" ];
    wants = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "${glueboxPkg}/bin/gluebox";
      Restart = "on-failure";
      RestartSec = 5;
      StateDirectory = "gluebox";
    };
    environment = {
      GLUEBOX_CONFIG = "/etc/gluebox/gluebox.toml";
    };
  };

  networking.firewall = {
    enable = true;
    allowedTCPPorts = [ 22 ];
    trustedInterfaces = [ "tailscale0" ];
  };

  environment.systemPackages = with pkgs; [
    curl
    htop
    mongosh
  ];

  system.stateVersion = "24.11";
}
