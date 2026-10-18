/*
 * Test driver for the Rust collision port: links the reference ctr-native
 * COLL.c and executes a script from stdin.
 *
 *   V <n>, then n lines "x y z flags"                      level vertices
 *   Q <n>, then n lines "i0..i8 flags minxyz maxxyz terrain bitshift d0..d9 drawOrderLow"
 *   B <n>, then n lines "flag id minxyz maxxyz" followed by
 *          "0 c0 c1 c2 c3" (branch) or "1 firstQuad numQuads" (leaf)
 *   S <searchFlags> <wanted> <ignored> <top xyz> <bottom xyz> <radius>
 *          COLL_SearchBSP_CallbackQUADBLK on the persistent scratchpad
 *   G <dividend> <bitshift> <lodshift> <v1 xyz> <v2 xyz> <v3 xyz>
 *          COLL_FIXED_TRIANGL_GetNormVec on scratchpad vertices 0..2
 *   Y <v1 xyz> <v2 xyz> <point xyz>
 *          COLL_FIXED_TRIANGL_Barycentrics
 *   M <searchFlags> <wanted> <ignored> <doorFlags> <current xyz> <next xyz> <radius>
 *          one sweep of COLL_MOVED_PlayerSearch (its lines setting up the
 *          scratchpad, then COLL_MOVED_BSPLEAF_TestQuadblocks over the BSP)
 *   F <quad index or -1> <triangle id>
 *          COLL_MOVED_FindScrub
 *
 * Every command prints one line; S and G print the relevant scratchpad state.
 * Pointers are printed as indices (-1 for null).
 */
#include "level.h"

int main(void)
{
	char cmd;

	memset(&ext, 0, sizeof(ext));
	sps.ptr_mesh_info = &mesh;

	while (scanf(" %c", &cmd) == 1)
	{
		if (read_level_block(cmd))
			continue;
		switch (cmd)
		{
		case 'S':
		{
			sps.Union.QuadBlockColl.searchFlags = rd();
			sps.Union.QuadBlockColl.quadFlagsWanted = (u32)rd();
			sps.Union.QuadBlockColl.quadFlagsIgnored = (u32)rd();
			SVec3 top, bottom;
			rdvec(&top);
			rdvec(&bottom);
			COLL_SearchBSP_CallbackQUADBLK(&top, &bottom, &sps, rd());
			pstate();
			break;
		}
		case 'G':
		{
			s16 dividend = rd();
			sps.collision.triNormalVecDividend = dividend;
			sps.collision.triNormalVecBitShift = rd();
			sps.collision.triNormalLodShift = rd();
			for (int i = 0; i < 3; i++)
				rdvec(&sps.bspSearchVert[i].pos);
			COLL_FIXED_TRIANGL_GetNormVec(&sps, &sps.bspSearchVert[0], &sps.bspSearchVert[1], &sps.bspSearchVert[2]);
			pstate();
			break;
		}
		case 'M':
		{
			SVec3 current, next;
			sps.Union.QuadBlockColl.searchFlags = rd();
			sps.Union.QuadBlockColl.quadFlagsWanted = (u32)rd();
			sps.Union.QuadBlockColl.quadFlagsIgnored = (u32)rd();
			sdata_static.doorAccessFlags = (u32)rd();
			rdvec(&current);
			rdvec(&next);
			s16 radius = rd();
			sps.Input1.hitRadius = radius;
			sps.Input1.hitRadiusSquared = radius * radius;
			sps.Union.QuadBlockColl.hitRadius = radius;
			sps.Union.QuadBlockColl.hitRadiusSquared = radius * radius;
			sps.boolDidTouchQuadblock = 0;
			sps.numTrianglesTested = 0;
			sps.boolDidTouchHitbox = 0;
			sps.resetOnly40 = 0;
			sps.hitFraction = COLL_FRACTION_ONE;
			sps.Union.QuadBlockColl.pos = current;
			sps.Input1.pos = next;
			if (next.x != current.x || next.y != current.y || next.z != current.z)
			{
				for (int axis = 0; axis < 3; axis++)
				{
					s16 c = ((s16 *)&current)[axis], n = ((s16 *)&next)[axis];
					s32 minC = c - radius, minN = n - radius, maxC = c + radius, maxN = n + radius;
					((s16 *)&sps.bbox.min)[axis] = minN < minC ? minN : minC;
					((s16 *)&sps.bbox.max)[axis] = maxC < maxN ? maxN : maxC;
				}
				sps.Union.QuadBlockColl.hitPos = sps.Input1.pos;
				COLL_SearchBSP_CallbackPARAM(mesh.bspRoot, &sps.bbox, COLL_MOVED_BSPLEAF_TestQuadblocks, &sps);
			}
			pstate();
			break;
		}
		case 'F':
		{
			int q = rd();
			COLL_MOVED_FindScrub(q < 0 ? NULL : &quads[q], rd(), &sps);
			pstate();
			break;
		}
		case 'Y':
		{
			SVec3 v1, v2, p, out;
			rdvec(&v1);
			rdvec(&v2);
			rdvec(&p);
			COLL_FIXED_TRIANGL_Barycentrics(&out, &v1, &v2, &p);
			pvec(out);
			printf("\n");
			break;
		}
		}
	}
	return 0;
}
