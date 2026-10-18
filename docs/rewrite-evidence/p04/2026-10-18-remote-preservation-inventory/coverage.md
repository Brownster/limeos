# Frozen remote public-surface coverage

Static declarations only; every required symbol maps to at least one preservation row.
No frozen code was imported/executed; this is not runtime/API/authentication proof.

| Symbol | Source | Rows |
| --- | --- | --- |
| `adapter:LegacyStorageCapabilityAdapter.candidates` | [storage_capability_adapters.py:116](/home/marc/Documents/github/pi-health/storage_capability_adapters.py:116) | RM-001, RM-027 |
| `api:add_mount` | [storage_plugins/__init__.py:352](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:352) | RM-024 |
| `api:delete_mount` | [storage_plugins/__init__.py:388](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:388) | RM-010, RM-018, RM-024 |
| `api:detect_mounts` | [storage_plugins/__init__.py:312](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:312) | RM-011, RM-024 |
| `api:get_mount_status` | [storage_plugins/__init__.py:439](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:439) | RM-012, RM-024 |
| `api:list_mounts` | [storage_plugins/__init__.py:336](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:336) | RM-024 |
| `api:list_plugins` | [storage_plugins/__init__.py:37](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:37) | RM-027 |
| `api:mount_filesystem` | [storage_plugins/__init__.py:405](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:405) | RM-006, RM-024 |
| `api:toggle_plugin` | [storage_plugins/__init__.py:43](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:43) | RM-011, RM-027 |
| `api:unmount_filesystem` | [storage_plugins/__init__.py:422](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:422) | RM-007, RM-024 |
| `api:update_mount` | [storage_plugins/__init__.py:370](/home/marc/Documents/github/pi-health/storage_plugins/__init__.py:370) | RM-024 |
| `base:RemoteMountPlugin.__init__` | [storage_plugins/remote_base.py:53](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:53) | RM-002 |
| `base:RemoteMountPlugin.add_mount` | [storage_plugins/remote_base.py:80](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:80) | RM-003 |
| `base:RemoteMountPlugin.get_install_instructions` | [storage_plugins/remote_base.py:158](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:158) | RM-001, RM-026 |
| `base:RemoteMountPlugin.get_mount` | [storage_plugins/remote_base.py:73](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:73) | RM-002 |
| `base:RemoteMountPlugin.is_installed` | [storage_plugins/remote_base.py:150](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:150) | RM-001, RM-026 |
| `base:RemoteMountPlugin.list_mounts_with_status` | [storage_plugins/remote_base.py:165](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:165) | RM-012, RM-030 |
| `base:RemoteMountPlugin.load_mounts` | [storage_plugins/remote_base.py:57](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:57) | RM-002, RM-013 |
| `base:RemoteMountPlugin.remove_mount` | [storage_plugins/remote_base.py:110](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:110) | RM-010, RM-018 |
| `base:RemoteMountPlugin.save_mounts` | [storage_plugins/remote_base.py:67](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:67) | RM-027 |
| `base:RemoteMountPlugin.update_mount` | [storage_plugins/remote_base.py:94](/home/marc/Documents/github/pi-health/storage_plugins/remote_base.py:94) | RM-004 |
| `disk:api_seedbox_get` | [disk_manager.py:239](/home/marc/Documents/github/pi-health/disk_manager.py:239) | RM-020 |
| `disk:api_seedbox_set` | [disk_manager.py:245](/home/marc/Documents/github/pi-health/disk_manager.py:245) | RM-021, RM-022 |
| `disk:default_seedbox_service` | [disk_manager.py:135](/home/marc/Documents/github/pi-health/disk_manager.py:135) | RM-020 |
| `helper:_ensure_dependencies` | [pihealth_helper.py:510](/home/marc/Documents/github/pi-health/pihealth_helper.py:510) | RM-026 |
| `helper:_get_rclone_unit_name` | [pihealth_helper.py:1777](/home/marc/Documents/github/pi-health/pihealth_helper.py:1777) | RM-023 |
| `helper:_get_sshfs_unit_names` | [pihealth_helper.py:2070](/home/marc/Documents/github/pi-health/pihealth_helper.py:2070) | RM-023 |
| `helper:_install_packages` | [pihealth_helper.py:489](/home/marc/Documents/github/pi-health/pihealth_helper.py:489) | RM-026 |
| `helper:_load_rclone_mounts` | [pihealth_helper.py:1782](/home/marc/Documents/github/pi-health/pihealth_helper.py:1782) | RM-013 |
| `helper:_load_sshfs_mounts` | [pihealth_helper.py:2051](/home/marc/Documents/github/pi-health/pihealth_helper.py:2051) | RM-002 |
| `helper:_read_rclone_remote` | [pihealth_helper.py:1838](/home/marc/Documents/github/pi-health/pihealth_helper.py:1838) | RM-013, RM-015 |
| `helper:_write_rclone_remote` | [pihealth_helper.py:1798](/home/marc/Documents/github/pi-health/pihealth_helper.py:1798) | RM-014, RM-015 |
| `helper:cmd_rclone_configure` | [pihealth_helper.py:1880](/home/marc/Documents/github/pi-health/pihealth_helper.py:1880) | RM-014, RM-015, RM-017, RM-019, RM-025, RM-026, RM-028 |
| `helper:cmd_rclone_list` | [pihealth_helper.py:1863](/home/marc/Documents/github/pi-health/pihealth_helper.py:1863) | RM-013, RM-019 |
| `helper:cmd_rclone_mount` | [pihealth_helper.py:2017](/home/marc/Documents/github/pi-health/pihealth_helper.py:2017) | RM-016, RM-030 |
| `helper:cmd_rclone_remove` | [pihealth_helper.py:2001](/home/marc/Documents/github/pi-health/pihealth_helper.py:2001) | RM-018, RM-025 |
| `helper:cmd_rclone_unmount` | [pihealth_helper.py:2028](/home/marc/Documents/github/pi-health/pihealth_helper.py:2028) | RM-016 |
| `helper:cmd_seedbox_configure` | [pihealth_helper.py:1695](/home/marc/Documents/github/pi-health/pihealth_helper.py:1695) | RM-021, RM-023, RM-025, RM-028, RM-030 |
| `helper:cmd_seedbox_disable` | [pihealth_helper.py:1765](/home/marc/Documents/github/pi-health/pihealth_helper.py:1765) | RM-022, RM-025 |
| `helper:cmd_sshfs_configure` | [pihealth_helper.py:2121](/home/marc/Documents/github/pi-health/pihealth_helper.py:2121) | RM-003, RM-004, RM-005, RM-008, RM-023, RM-025, RM-026, RM-028 |
| `helper:cmd_sshfs_list` | [pihealth_helper.py:2089](/home/marc/Documents/github/pi-health/pihealth_helper.py:2089) | RM-002, RM-012 |
| `helper:cmd_sshfs_mount` | [pihealth_helper.py:2311](/home/marc/Documents/github/pi-health/pihealth_helper.py:2311) | RM-006, RM-030 |
| `helper:cmd_sshfs_remove` | [pihealth_helper.py:2271](/home/marc/Documents/github/pi-health/pihealth_helper.py:2271) | RM-009, RM-010, RM-025 |
| `helper:cmd_sshfs_unmount` | [pihealth_helper.py:2333](/home/marc/Documents/github/pi-health/pihealth_helper.py:2333) | RM-007 |
| `manager:is_enabled` | [plugin_manager.py:102](/home/marc/Documents/github/pi-health/plugin_manager.py:102) | RM-001 |
| `manager:list_plugins` | [plugin_manager.py:124](/home/marc/Documents/github/pi-health/plugin_manager.py:124) | RM-001, RM-027 |
| `manager:load_plugins_config` | [plugin_manager.py:88](/home/marc/Documents/github/pi-health/plugin_manager.py:88) | RM-027 |
| `page:@file` | [frontend/src/pages/mounts-page.tsx:1](/home/marc/Documents/github/pi-health/frontend/src/pages/mounts-page.tsx:1) | RM-024, RM-029 |
| `paths:@file` | [runtime_paths.py:1](/home/marc/Documents/github/pi-health/runtime_paths.py:1) | RM-020 |
| `rclone:RclonePlugin.add_mount` | [storage_plugins/rclone_plugin.py:156](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:156) | RM-014 |
| `rclone:RclonePlugin.disable_automount` | [storage_plugins/rclone_plugin.py:144](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:144) | RM-017 |
| `rclone:RclonePlugin.enable_automount` | [storage_plugins/rclone_plugin.py:132](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:132) | RM-017 |
| `rclone:RclonePlugin.get_mount_status` | [storage_plugins/rclone_plugin.py:112](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:112) | RM-019 |
| `rclone:RclonePlugin.get_schema` | [storage_plugins/rclone_plugin.py:23](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:23) | RM-014, RM-019 |
| `rclone:RclonePlugin.mount` | [storage_plugins/rclone_plugin.py:94](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:94) | RM-016 |
| `rclone:RclonePlugin.unmount` | [storage_plugins/rclone_plugin.py:103](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:103) | RM-016 |
| `rclone:RclonePlugin.update_mount` | [storage_plugins/rclone_plugin.py:178](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:178) | RM-015 |
| `rclone:RclonePlugin.validate_mount_config` | [storage_plugins/rclone_plugin.py:59](/home/marc/Documents/github/pi-health/storage_plugins/rclone_plugin.py:59) | RM-014 |
| `registry:get_registry` | [storage_plugins/registry.py:119](/home/marc/Documents/github/pi-health/storage_plugins/registry.py:119) | RM-001 |
| `registry:init_plugins` | [storage_plugins/registry.py:128](/home/marc/Documents/github/pi-health/storage_plugins/registry.py:128) | RM-001 |
| `seed:SeedboxService._default_config` | [seedbox_service.py:118](/home/marc/Documents/github/pi-health/seedbox_service.py:118) | RM-020 |
| `seed:SeedboxService._validate_enabled` | [seedbox_service.py:128](/home/marc/Documents/github/pi-health/seedbox_service.py:128) | RM-021 |
| `seed:SeedboxService.config` | [seedbox_service.py:41](/home/marc/Documents/github/pi-health/seedbox_service.py:41) | RM-020 |
| `seed:SeedboxService.configure` | [seedbox_service.py:66](/home/marc/Documents/github/pi-health/seedbox_service.py:66) | RM-021, RM-022 |
| `seed:SeedboxService.is_mounted` | [seedbox_service.py:60](/home/marc/Documents/github/pi-health/seedbox_service.py:60) | RM-020 |
| `seed:SeedboxService.save` | [seedbox_service.py:52](/home/marc/Documents/github/pi-health/seedbox_service.py:52) | RM-021 |
| `seed:SeedboxService.state` | [seedbox_service.py:56](/home/marc/Documents/github/pi-health/seedbox_service.py:56) | RM-020 |
| `sshfs:SSHFSPlugin._detect_sshfs_mounts` | [storage_plugins/sshfs_plugin.py:260](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:260) | RM-011 |
| `sshfs:SSHFSPlugin._parse_sshfs_source` | [storage_plugins/sshfs_plugin.py:289](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:289) | RM-011 |
| `sshfs:SSHFSPlugin.add_mount` | [storage_plugins/sshfs_plugin.py:180](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:180) | RM-003 |
| `sshfs:SSHFSPlugin.disable_automount` | [storage_plugins/sshfs_plugin.py:171](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:171) | RM-009 |
| `sshfs:SSHFSPlugin.enable_automount` | [storage_plugins/sshfs_plugin.py:159](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:159) | RM-008 |
| `sshfs:SSHFSPlugin.get_mount_status` | [storage_plugins/sshfs_plugin.py:117](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:117) | RM-012 |
| `sshfs:SSHFSPlugin.get_schema` | [storage_plugins/sshfs_plugin.py:26](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:26) | RM-003, RM-005, RM-028 |
| `sshfs:SSHFSPlugin.import_existing_mounts` | [storage_plugins/sshfs_plugin.py:237](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:237) | RM-011 |
| `sshfs:SSHFSPlugin.mount` | [storage_plugins/sshfs_plugin.py:99](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:99) | RM-006 |
| `sshfs:SSHFSPlugin.remove_mount` | [storage_plugins/sshfs_plugin.py:230](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:230) | RM-010 |
| `sshfs:SSHFSPlugin.unmount` | [storage_plugins/sshfs_plugin.py:108](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:108) | RM-007 |
| `sshfs:SSHFSPlugin.update_mount` | [storage_plugins/sshfs_plugin.py:202](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:202) | RM-004 |
| `sshfs:SSHFSPlugin.validate_mount_config` | [storage_plugins/sshfs_plugin.py:64](/home/marc/Documents/github/pi-health/storage_plugins/sshfs_plugin.py:64) | RM-003, RM-005 |
| `ui:@file` | [frontend/src/lib/mounts.ts:1](/home/marc/Documents/github/pi-health/frontend/src/lib/mounts.ts:1) | RM-024, RM-029 |
