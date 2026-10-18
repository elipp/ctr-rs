/*
 * Test driver for the Rust vehicle port: links the reference ctr-native
 * COLL.c, VehCalc.c, VehAfterColl.c and VehPhysProc.c and runs
 * COLL_MOVED_PlayerSearch on a driver described by a script on stdin.
 *
 *   V, Q, B   the level, as in the coll driver
 *   K <7 x "flags speedLimit impactAngle">
 *             the scrub table (data.MetaDataScrub)
 *   T <1024 x "sin cos">
 *             the quarter-turn trig table (data.trigApprox)
 *   P <elapsedMS> <light matrix: 9 values> <doorFlags> <driver fields...>
 *             loads the driver (field order: `fields` below), runs
 *             COLL_MOVED_PlayerSearch with the scratchpad kept from earlier
 *             commands, and prints the driver, the effects it requested and
 *             the scratchpad state
 */
#include <stdarg.h>
#include "../../../coll/tests/reference/level.h"

struct Thread thread;
struct Instance inst;
struct Driver d;
struct GameTracker gGT;
struct Level level;

static char effects[1024];

static void effect(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vsnprintf(effects + strlen(effects), sizeof(effects) - strlen(effects), fmt, ap);
	va_end(ap);
}

int OtherFX_Play_LowLevel(u32 soundID, u8 boolAntiSpam, u32 flags)
{
	effect(" S %u %u", soundID, (flags >> HOWL_SFX_ECHO_SHIFT) & 0xff);
	return 0;
}

void Voiceline_RequestPlay(u32 voiceID, u32 characterID, u32 priority)
{
	effect(" V %u %u %u", voiceID, characterID, priority);
}

void GAMEPAD_JogCon1(struct Driver *drv, s32 val, u16 timeMS)
{
	effect(" J %d %d %d", drv->driverID, val, timeMS);
}

void GAMEPAD_ShockFreq(struct Driver *drv, s32 frame, s32 val)
{
	effect(" F %d %d %d", drv->driverID, frame, val);
}

void GAMEPAD_ShockForce1(struct Driver *drv, s32 frame, s32 val)
{
	effect(" G %d %d %d", drv->driverID, frame, val);
}

enum { S8, U8, S16, U16, S32, QUAD, MATRIX_ARRAY, PHYS_STATE };

struct Field
{
	void *p;
	int type;
};

#define F(member, type) { &(member), type }

static struct Field fields[] = {
	F(d.driverID, U8), F(d.kartState, U8), F(d.simpTurnState, S8),
	F(d.posCurr.x, S32), F(d.posCurr.y, S32), F(d.posCurr.z, S32),
	F(d.velocity.x, S32), F(d.velocity.y, S32), F(d.velocity.z, S32),
	F(d.originToCenter.x, S32), F(d.originToCenter.y, S32), F(d.originToCenter.z, S32),
	F(d.collisionFlags, S16), F(d.stepFlagSet, S32), F(d.actionsFlagSet, S32), F(d.actionsFlagSetPrevFrame, S32),
	F(d.currBlockTouching, QUAD), F(d.underDriver, QUAD), F(d.lastValid, QUAD),
	F(d.normalVecUP.x, S16), F(d.normalVecUP.y, S16), F(d.normalVecUP.z, S16),
	F(d.AxisAngle1_normalVec.x, S16), F(d.AxisAngle1_normalVec.y, S16), F(d.AxisAngle1_normalVec.z, S16),
	F(d.spsHitPos.x, S16), F(d.spsHitPos.y, S16), F(d.spsHitPos.z, S16),
	F(d.spsNormalVec.x, S16), F(d.spsNormalVec.y, S16), F(d.spsNormalVec.z, S16),
	F(d.posWallColl.x, S16), F(d.posWallColl.y, S16), F(d.posWallColl.z, S16),
	F(d.vShiftCount, S16), F(d.speed, S16), F(d.speedApprox, S16), F(d.baseSpeed, S16), F(d.fireSpeed, S16),
	F(d.jumpHeightCurr, S16), F(d.xSpeed, S32), F(d.ySpeed, S32), F(d.zSpeed, S32), F(d.speedometerNeedleValue, S16),
	F(d.reserves, S16), F(d.turbo_outsideTimer, S16), F(d.turbo_MeterRoomLeft, S16),
	F(d.wallRubTimer, S16), F(d.wallRubSpeedLimit, S16),
	F(d.angle, S16), F(d.rotCurr.w, S16), F(d.turnAngleCurr, S16), F(d.turnAngleLerpVel, S16),
	F(d.turnWobbleAngle, S16), F(d.turnWobbleVelocity, S16), F(d.turnWobbleTimer, S16), F(d.ampTurnState, S16),
	F(d.numFramesSpentSteering, S16), F(d.rotationSpinRate, S16), F(d.VehFire_AudioCooldown, S16),
	F(d.Screen_OffsetY, S8), F(d.distanceFromGround, S16), F(d.reserved_0x40e, S16),
	F(d.jumpSquishStretch, S16), F(d.jumpSquishStretch2, S16),
	F(inst.animIndex, U8), F(inst.animFrame, S16), F(inst.scale.x, S16), F(inst.scale.y, S16),
	F(d.matrixArray, MATRIX_ARRAY), F(d.matrixIndex, U8),
	F(d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR], PHYS_STATE),
};

#define NUM_FIELDS ((int)(sizeof(fields) / sizeof(fields[0])))

static void load_driver(void)
{
	for (int i = 0; i < NUM_FIELDS; i++)
	{
		int v = rd();
		void *p = fields[i].p;
		switch (fields[i].type)
		{
		case S8: *(s8 *)p = v; break;
		case U8: *(u8 *)p = v; break;
		case S16: *(s16 *)p = v; break;
		case U16: *(u16 *)p = v; break;
		case S32: *(s32 *)p = v; break;
		case QUAD: *(struct QuadBlock **)p = v < 0 ? NULL : &quads[v]; break;
		case MATRIX_ARRAY: *(u8 *)p = v; break;
		case PHYS_STATE: *(DriverFunc *)p = v ? (DriverFunc)VehPhysProc_SlamWall_PhysLinear : NULL; break;
		}
	}
}

static void print_driver(void)
{
	for (int i = 0; i < NUM_FIELDS; i++)
	{
		void *p = fields[i].p;
		switch (fields[i].type)
		{
		case S8: printf(" %d", *(s8 *)p); break;
		case U8: printf(" %u", *(u8 *)p); break;
		case S16: printf(" %d", *(s16 *)p); break;
		case U16: printf(" %u", *(u16 *)p); break;
		case S32: printf(" %d", *(s32 *)p); break;
		case QUAD: printf(" %d", quadIndex(*(struct QuadBlock **)p)); break;
		case MATRIX_ARRAY: printf(" %u", *(u8 *)p); break;
		case PHYS_STATE: printf(" %d", *(DriverFunc *)p == (DriverFunc)VehPhysProc_SlamWall_PhysLinear); break;
		}
	}
}

int main(void)
{
	char cmd;

	memset(&ext, 0, sizeof(ext));
	for (int i = 0; i < 8; i++)
		data.characterIDs[i] = i;
	sdata_static.gGT = &gGT;
	gGT.level1 = &level;
	gGT.numPlyrCurrGame = 1;
	level.ptr_mesh_info = &mesh;
	thread.inst = &inst;
	d.instSelf = &inst;

	while (scanf(" %c", &cmd) == 1)
	{
		if (read_level_block(cmd))
			continue;
		switch (cmd)
		{
		case 'K':
			for (int i = 0; i < 7; i++)
			{
				data.MetaDataScrub[i].flags = rd();
				data.MetaDataScrub[i].speedLimit = rd();
				data.MetaDataScrub[i].impactAngle = rd();
			}
			break;
		case 'T':
			for (int i = 0; i < 0x400; i++)
			{
				data.trigApprox[i].sin = rd();
				data.trigApprox[i].cos = rd();
			}
			break;
		case 'P':
		{
			gGT.elapsedTimeMS = rd();
			u32 m[9];
			for (int i = 0; i < 9; i++)
				m[i] = (u16)rd();
			CTC2(m[0] | (m[1] << 16), 8);
			CTC2(m[2] | (m[3] << 16), 9);
			CTC2(m[4] | (m[5] << 16), 10);
			CTC2(m[6] | (m[7] << 16), 11);
			CTC2(m[8], 12);
			sdata_static.doorAccessFlags = (u32)rd();
			load_driver();

			effects[0] = 0;
			memcpy(scratchpad + 0x108, &ext, sizeof(ext));
			/* ext's pointers into its own vertices become the scratchpad's. */
			struct ScratchpadStruct *pad = (struct ScratchpadStruct *)(scratchpad + 0x108);
			struct BspSearchVertex **padTri[3] = { &pad->hitBspSearchTriangle.v0, &pad->hitBspSearchTriangle.v1, &pad->hitBspSearchTriangle.v2 };
			for (int i = 0; i < 3; i++)
				if (*padTri[i])
					*padTri[i] = pad->bspSearchVert + (*padTri[i] - sps.bspSearchVert);
			COLL_MOVED_PlayerSearch(&thread, &d);
			memcpy(&ext, scratchpad + 0x108, sizeof(ext));
			/* Pointers into the scratchpad's own vertices, as indices into ext's. */
			struct BspSearchVertex *base = pad->bspSearchVert;
			struct BspSearchVertex **tri[3] = { &sps.hitBspSearchTriangle.v0, &sps.hitBspSearchTriangle.v1, &sps.hitBspSearchTriangle.v2 };
			for (int i = 0; i < 3; i++)
				if (*tri[i])
					*tri[i] = sps.bspSearchVert + (*tri[i] - base);

			print_driver();
			printf(" |%s | ", effects);
			pstate();
			break;
		}
		}
	}
	return 0;
}
