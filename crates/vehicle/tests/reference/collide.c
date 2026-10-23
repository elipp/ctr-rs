/*
 * Test driver for the Rust vehicle port's kart-against-kart collisions: links
 * the reference ctr-native PROC.c, VehPhysForce.c and VehPhysCrash.c and runs
 * `VehPhysForce_CollideDrivers` of the first driver of a thread list.
 *
 *   G <frame> <lastFeedbackFrame>
 *   D <i> <driverID> <kartState> <simpTurnState> <pos x3> <velocity x3> <accel x3> <collisionWeight>
 *     <actionsFlagSet> <reserves> <invincibleTimer> <forcedJumpType> <disabled> <inst t x3>
 *             loads driver i (drivers are a thread list in the order of i)
 *   X <n>     runs CollideDrivers of driver 0 with n drivers in the list; prints
 *             for each driver: velocity x3, actionsFlagSet, forcedJumpType, pendingDamageType,
 *             pendingDamageReasonByte, the attacker's driverID or -1; then the effects
 *             and the last feedback frame
 */
#include <stdarg.h>
#include "../../../coll/tests/reference/level.h"

#define MAX 4

struct Thread threads[MAX];
struct Instance insts[MAX];
struct Driver drivers[MAX];
struct GameTracker gGT;

static char effects[1024];

static void effect(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vsnprintf(effects + strlen(effects), sizeof(effects) - strlen(effects), fmt, ap);
	va_end(ap);
}

void OtherFX_DriverCrashing(u32 boolEcho, u32 volume)
{
	effect(" C %u %u", boolEcho, volume);
}

int OtherFX_Play(u32 soundID, int flags)
{
	effect(" P %u", soundID);
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

int main(void)
{
	char cmd;

	memset(&ext, 0, sizeof(ext));
	sdata_static.gGT = &gGT;
	for (int i = 0; i < MAX; i++)
	{
		threads[i].inst = &insts[i];
		threads[i].object = &drivers[i];
		threads[i].modelIndex = DYNAMIC_PLAYER;
		threads[i].driverHitRadius = THREAD_DRIVER_HIT_RADIUS;
		threads[i].driverHitRadiusSquared = THREAD_DRIVER_HIT_RADIUS_SQUARED;
		insts[i].thread = &threads[i];
		drivers[i].instSelf = &insts[i];
	}
	for (int i = 0; i < 8; i++)
		data.characterIDs[i] = i;

	while (scanf(" %c", &cmd) == 1)
	{
		switch (cmd)
		{
		case 'G':
			gGT.frameTimer_MainFrame_ResetDB = rd();
			sdata_static.audioDefaults[8] = rd();
			break;
		case 'D':
		{
			struct Driver *d = &drivers[rd()];
			d->driverID = rd();
			d->kartState = rd();
			d->simpTurnState = rd();
			d->posCurr.x = rd();
			d->posCurr.y = rd();
			d->posCurr.z = rd();
			d->velocity.x = rd();
			d->velocity.y = rd();
			d->velocity.z = rd();
			d->accel.x = rd();
			d->accel.y = rd();
			d->accel.z = rd();
			d->const_CollisionWeight = rd();
			d->actionsFlagSet = rd();
			d->reserves = rd();
			d->invincibleTimer = rd();
			d->forcedJumpType = rd();
			d->instSelf->thread->flags = rd() ? THREAD_FLAG_DISABLE_COLLISION : 0;
			d->instSelf->matrix.t[0] = rd();
			d->instSelf->matrix.t[1] = rd();
			d->instSelf->matrix.t[2] = rd();
			d->pendingDamageType = 0;
			d->pendingDamageReasonByte = 0;
			d->pendingDamageAttacker = NULL;
			d->stepFlagSet = 0;
			d->collisionFlags = 0;
			break;
		}
		case 'X':
		{
			int n = rd();
			for (int i = 0; i < MAX; i++)
				threads[i].siblingThread = i + 1 < n ? &threads[i + 1] : NULL;
			gGT.threadBuckets[ROBOT].thread = NULL;
			effects[0] = 0;
			VehPhysForce_CollideDrivers(&threads[0], &drivers[0]);
			for (int i = 0; i < n; i++)
			{
				struct Driver *d = &drivers[i];
				printf("%d %d %d %d %d %d %d %d %d |", d->velocity.x, d->velocity.y, d->velocity.z, d->actionsFlagSet,
				       d->forcedJumpType, d->pendingDamageType, d->pendingDamageReasonByte,
				       d->pendingDamageAttacker ? d->pendingDamageAttacker->driverID : -1, 0);
			}
			printf(" %s | %d\n", effects, sdata_static.audioDefaults[8]);
			break;
		}
		}
	}
	return 0;
}
