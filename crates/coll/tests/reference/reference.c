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
#include <common.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

u8 scratchpad[CTR_SCRATCHPAD_SIZE];
u8 *gCTRNativeScratchpadBase = scratchpad;
struct Data data;
struct sData sdata_static;

static struct LevVertex *verts;
static struct QuadBlock *quads;
static struct BSP *bsps;
static struct mesh_info mesh;
static struct ScratchpadStructExtended ext;
#define sps (ext.scratchpadStruct)
static int numQuads;
/* Stands in for the null quad that empty BSP leaves point at: never matches. */
static struct QuadBlock dummyQuad;

static int rd(void)
{
	int v;
	if (scanf("%d", &v) != 1)
		exit(2);
	return v;
}

static void rdvec(SVec3 *v)
{
	v->x = rd();
	v->y = rd();
	v->z = rd();
}

static int quadIndex(struct QuadBlock *q)
{
	return (q && q != &dummyQuad) ? (int)(q - quads) : -1;
}

static int vertIndex(struct LevVertex *v)
{
	return v ? (int)(v - verts) : -1;
}

static int searchVertIndex(struct BspSearchVertex *v)
{
	return v ? (int)(v - sps.bspSearchVert) : -1;
}

static void pvec(SVec3 v)
{
	printf(" %d %d %d", v.x, v.y, v.z);
}

static void pplane(struct CollPlane p)
{
	pvec(p.normal);
	printf(" %d", p.halfDistance);
}

static void pstate(void)
{
	printf("%d %d %u %d", sps.numTrianglesTested, sps.boolDidTouchQuadblock, sps.collision.stepFlags, sps.hitFraction);
	pvec(sps.Union.QuadBlockColl.pos);
	pvec(sps.Union.QuadBlockColl.hitPos);
	pvec(sps.hit.hitPos);
	pplane(sps.hit.plane);
	printf(" %d", quadIndex(sps.hit.ptrQuadblock));
	pvec(sps.candidate.hitPos);
	printf(" %d", sps.candidate.normalAxis);
	pplane(sps.candidate.plane);
	printf(" %d", quadIndex(sps.candidate.ptrQuadblock));
	printf(" %d %d", sps.hitBarycentrics.v1, sps.hitBarycentrics.v2);
	printf(" %d %d %d", vertIndex(sps.hitLevelTriangle.v0), vertIndex(sps.hitLevelTriangle.v1), vertIndex(sps.hitLevelTriangle.v2));
	printf(" %d %d %d %d", sps.collision.triNormalVecDividend, sps.collision.triNormalVecBitShift, sps.collision.triNormalLodShift,
	       sps.quadSecondTriIndexA * 65536 + sps.quadSecondTriIndexB);
	pvec(sps.candidate.pushOut);
	pvec(sps.hit.pushOut);
	pvec(sps.candidateDelta);
	printf(" %d %d %d %d %d", sps.candidate.triangleID, sps.hit.triangleID, sps.hit.reorderResult, sps.hit.normalAxis, sps.Input1.scrubDepth);
	printf(" %d %d %d", searchVertIndex(sps.hitBspSearchTriangle.v0), searchVertIndex(sps.hitBspSearchTriangle.v1), searchVertIndex(sps.hitBspSearchTriangle.v2));
	printf(" %d %d", sps.Union.QuadBlockColl.searchFlags, ext.numTriangles);
	for (int i = 0; i < ext.numTriangles; i++)
		printf(" %d %d %d", quadIndex(ext.bspSearchTriangle[i].quadblock), ext.bspSearchTriangle[i].triangleID, ext.bspSearchTriangle[i].scrubDepth);
	for (int i = 0; i < 9; i++)
	{
		struct BspSearchVertex *v = &sps.bspSearchVert[i];
		pvec(v->pos);
		printf(" %d %d", v->normalAxis, vertIndex(v->pLevelVertex));
		pplane(v->plane);
	}
	printf("\n");
}

int main(void)
{
	char cmd;

	memset(&ext, 0, sizeof(ext));
	sps.ptr_mesh_info = &mesh;

	while (scanf(" %c", &cmd) == 1)
	{
		switch (cmd)
		{
		case 'V':
		{
			int n = rd();
			verts = calloc(n, sizeof(*verts));
			for (int i = 0; i < n; i++)
			{
				rdvec(&verts[i].pos);
				verts[i].flags = rd();
			}
			mesh.numVertex = n;
			mesh.ptrVertexArray = verts;
			break;
		}
		case 'Q':
		{
			int n = rd();
			numQuads = n;
			quads = calloc(n, sizeof(*quads));
			for (int i = 0; i < n; i++)
			{
				struct QuadBlock *q = &quads[i];
				for (int k = 0; k < 9; k++)
					q->index[k] = rd();
				q->quadFlags = rd();
				rdvec(&q->bbox.min);
				rdvec(&q->bbox.max);
				q->terrain_type = rd();
				q->triNormalVecBitShift = rd();
				for (int k = 0; k < 10; k++)
					q->triNormalVecDividend[k] = rd();
				q->draw_order_low = (u32)rd();
			}
			mesh.numQuadBlock = n;
			mesh.ptrQuadBlockArray = quads;
			break;
		}
		case 'B':
		{
			int n = rd();
			bsps = calloc(n, sizeof(*bsps));
			for (int i = 0; i < n; i++)
			{
				struct BSP *b = &bsps[i];
				b->flag = rd();
				b->id = rd();
				rdvec(&b->box.min);
				rdvec(&b->box.max);
				if (rd() == 0)
				{
					for (int k = 0; k < 4; k++)
						b->data.branch.childID[k] = rd();
				}
				else
				{
					int first = rd();
					int count = rd();
					b->data.leaf.ptrQuadBlockArray = count ? &quads[first] : &dummyQuad;
					b->data.leaf.numQuads = count;
				}
			}
			mesh.numBspNodes = n;
			mesh.bspRoot = bsps;
			break;
		}
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
