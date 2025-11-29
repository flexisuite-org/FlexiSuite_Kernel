-- Row Level Security policies for multi-tenant isolation.
-- Run AFTER Prisma migrations. These rely on the session GUC `flexi.current_group` being set per request.

-- Enable RLS
ALTER TABLE "Group" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "GroupMember" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "Role" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "Permission" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "RolePermission" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "AppInstall" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "EntityRecord" ENABLE ROW LEVEL SECURITY;
ALTER TABLE "AuditLog" ENABLE ROW LEVEL SECURITY;

-- Policies (read/write restricted to current group)
CREATE POLICY group_select ON "Group" USING (id = current_setting('flexi.current_group', true));
CREATE POLICY group_member_access ON "GroupMember" USING ("groupId" = current_setting('flexi.current_group', true));
CREATE POLICY role_access ON "Role" USING ("groupId" = current_setting('flexi.current_group', true));
CREATE POLICY permission_access ON "Permission" USING ("groupId" IS NULL OR "groupId" = current_setting('flexi.current_group', true));
CREATE POLICY role_permission_access ON "RolePermission" USING (
  (SELECT "groupId" FROM "Role" r WHERE r.id = "roleId") = current_setting('flexi.current_group', true)
);
CREATE POLICY app_install_access ON "AppInstall" USING ("groupId" = current_setting('flexi.current_group', true));
CREATE POLICY entity_record_access ON "EntityRecord" USING ("groupId" = current_setting('flexi.current_group', true));
CREATE POLICY auditlog_access ON "AuditLog" USING (
  "groupId" IS NULL OR "groupId" = current_setting('flexi.current_group', true)
);

-- Restrict inserts/updates to same policy conditions
ALTER TABLE "Group" FORCE ROW LEVEL SECURITY;
ALTER TABLE "GroupMember" FORCE ROW LEVEL SECURITY;
ALTER TABLE "Role" FORCE ROW LEVEL SECURITY;
ALTER TABLE "Permission" FORCE ROW LEVEL SECURITY;
ALTER TABLE "RolePermission" FORCE ROW LEVEL SECURITY;
ALTER TABLE "AppInstall" FORCE ROW LEVEL SECURITY;
ALTER TABLE "EntityRecord" FORCE ROW LEVEL SECURITY;
ALTER TABLE "AuditLog" FORCE ROW LEVEL SECURITY;
