/*
 * Shared by the C drivers of the coll and vehicle differential tests: the
 * scratchpad, the level description (`V`, `Q` and `B` blocks, written by the
 * Rust side's `describe_level`) and the scratchpad state dump.
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


/* Handles the level description commands; returns 1 if `cmd` was one. */
static int read_level_block(char cmd)
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
		return 1;
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
		return 1;
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
		return 1;
		}
	}
	return 0;
}
