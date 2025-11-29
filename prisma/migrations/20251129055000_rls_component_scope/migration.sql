-- Enable RLS and add tenant policies for multi-tenant tables

-- GroupMember
ALTER TABLE "GroupMember" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "groupmember_tenant" ON "GroupMember"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));

-- Role
ALTER TABLE "Role" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "role_tenant" ON "Role"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));

-- Permission (nullable groupId -> only scoped rows are accessible)
ALTER TABLE "Permission" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "permission_tenant" ON "Permission"
  USING ("groupId" IS NULL OR "groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" IS NULL OR "groupId" = current_setting('flexi.current_group'));

-- AppInstall
ALTER TABLE "AppInstall" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "appinstall_tenant" ON "AppInstall"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));

-- EntityRecord
ALTER TABLE "EntityRecord" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "entityrecord_tenant" ON "EntityRecord"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));

-- ComponentPackage (ownerGroupId)
ALTER TABLE "ComponentPackage" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "componentpackage_tenant" ON "ComponentPackage"
  USING ("ownerGroupId" = current_setting('flexi.current_group'))
  WITH CHECK ("ownerGroupId" = current_setting('flexi.current_group'));

-- ComponentDependency follows ComponentPackage ownership via FK; no policy needed

-- ComponentInstall
ALTER TABLE "ComponentInstall" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "componentinstall_tenant" ON "ComponentInstall"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));

-- RolloutRule inherits tenant via ComponentInstall FK; no direct policy required.
