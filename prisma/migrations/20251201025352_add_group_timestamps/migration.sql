-- AlterTable
ALTER TABLE "Group" ALTER COLUMN "updatedAt" DROP DEFAULT;

-- AddForeignKey
ALTER TABLE "AccountInvite" ADD CONSTRAINT "AccountInvite_initialGroupId_fkey" FOREIGN KEY ("initialGroupId") REFERENCES "Group"("id") ON DELETE SET NULL ON UPDATE CASCADE;
