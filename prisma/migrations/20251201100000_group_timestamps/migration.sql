-- Add createdAt/updatedAt to Group to match prisma/schema.prisma
-- IF NOT EXISTS を付けておくことで、
-- - すでに手元DBにカラムがある場合
-- - まだ何もない新規DB（CIなど）
-- の両方で安全に適用できるようにする。
ALTER TABLE "Group"
  ADD COLUMN IF NOT EXISTS "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  ADD COLUMN IF NOT EXISTS "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP;
