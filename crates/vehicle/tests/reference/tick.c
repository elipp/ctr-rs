/*
 * Test driver for the Rust vehicle port's tick: links the reference ctr-native
 * COLL.c, VehCalc.c, VehAfterColl.c, VehPhysProc.c, VehPhysGeneral.c,
 * VehPhysForce.c, VehPhysCrash.c and VehPhysJoystick.c and runs the stages of
 * MainFrame's loop over the driver's function table, as the Rust `tick` does.
 *
 *   V, Q, B   the level, as in the coll driver
 *   E <numQuads x "mulNormVecY checkpointIndex"> <jumpVerticalSpeedCap>
 *     <configFlags> <numRestartPoints x "distToFinish nextIndexForward">
 *             the parts of the level the coll driver's blocks leave out
 *   K <7 x "flags speedLimit impactAngle">     the scrub table
 *   T <1024 x "sin cos">                       the quarter-turn trig table
 *   R <21 x "flags speedMultiplier slowUntilSpeed counterSteerRatio
 *            turnLeanScale groundFrictionScale turnAngleScale
 *            turnResponseScale sound">         the terrain table
 *   A <8 x numFrames>                          the frames of the model's animations
 *   S <65 values>                              the engine class's constants
 *   C <characterID>                            the driver's character
 *   N         prints the names of the driver fields, in order
 *   D <driver fields...>
 *             loads the driver (field order: `fields` below)
 *   I <reserves> <type> <fireLevel>
 *             a boost before the next tick's stages, where
 *             VehPickupItem_ShootOnCirclePress fires a turbo item
 *   X <elapsedMS> <elapsedEventTime> <timer> <endOfRace> <buttonsHeld>
 *     <buttonsTapped> <stickLX> <stickLY> <stickRY>
 *             runs one tick; prints the driver, the effects it requested and
 *             a few values of the scratchpad
 *
 * Everything of the driver's states is the real code: driving, drifting,
 * spinning out, the crash and the V-shift freeze; only the stages that are
 * presentation are left out. The exhaust fire is the real VehTurbo.c too: its
 * thread is born in the TURBO bucket, ticks after the drivers' stages and dies
 * in PROC_CheckAllForDead, before the HUD's jump meter, as in MainFrame.
 */
#include <stdarg.h>
#include "../../../coll/tests/reference/level.h"
#include <psx/gtereg.h>

struct Thread thread;
struct Instance inst;
struct Driver d;
struct GameTracker gGT;
struct Level level;
struct GamepadSystem pads;
struct Model model;
/* The baked animation matrices are not ported; the C animation only needs somewhere to read them from. */
struct MatrixND bakedDummy[256];
struct ModelHeader modelHeader;
struct ModelAnim *modelAnims[8];
struct ModelAnim modelAnimData[8];
struct CheckpointNode *restartPoints;

static char effects[1024];
static int maskGrab;
/* While the turbo thread ticks: its rumble is not ported. */
static int inTurboTick;

/* The one turbo thread (one driver), in gGT.threadBuckets[TURBO] while it lives. */
struct Thread turboThread;
struct Instance turboInst1, turboInst2;
struct Turbo turboObj;

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

int OtherFX_Play(u32 soundID, int flags)
{
	return OtherFX_Play_LowLevel(soundID & 0xffff, flags & 0xff, HOWL_SFX_DEFAULT_FLAGS);
}

void OtherFX_Play_Echo(u32 soundID, int flags, int echoFlag)
{
	OtherFX_Play_LowLevel(soundID & 0xffff, flags & 0xff, echoFlag ? HOWL_SFX_DEFAULT_FLAGS | HOWL_SFX_ECHO_FLAG : HOWL_SFX_DEFAULT_FLAGS);
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
	if (inTurboTick)
		return;
	effect(" F %d %d %d", drv->driverID, frame, val);
}

void GAMEPAD_ShockForce1(struct Driver *drv, s32 frame, s32 val)
{
	effect(" G %d %d %d", drv->driverID, frame, val);
}

/* The turbo fire's thread and its two instances (VehFire_Increment's is the only birth here). */
struct Instance *INSTANCE_BirthWithThread(s32 modelID, const char *name, s32 poolType, s32 bucket, void *funcThTick, s32 objSize, struct Thread *parent)
{
	memset(&turboThread, 0, sizeof(turboThread));
	memset(&turboInst1, 0, sizeof(turboInst1));
	memset(&turboObj, 0, sizeof(turboObj));
	turboThread.object = &turboObj;
	turboThread.inst = &turboInst1;
	turboThread.funcThTick = funcThTick;
	turboInst1.thread = &turboThread;
	gGT.threadBuckets[TURBO].thread = &turboThread;
	return &turboInst1;
}

struct Instance *INSTANCE_Birth3D(struct Model *m, const char *name, struct Thread *th)
{
	memset(&turboInst2, 0, sizeof(turboInst2));
	turboInst2.thread = th;
	return &turboInst2;
}

void INSTANCE_Death(struct Instance *inst)
{
}

/* The fire's roar (slot 3 of the driver's sounds); id -1 stops it. */
void OtherFX_RecycleNew(u32 *soundID_Count, u32 newSoundID, u32 modifyFlags)
{
	effect(" R %d %u %u %u", (s32)newSoundID, (modifyFlags >> HOWL_SFX_VOLUME_SHIFT) & 0xff,
	       (modifyFlags >> HOWL_SFX_DISTORTION_SHIFT) & 0xff, (modifyFlags >> HOWL_SFX_ECHO_SHIFT) & 1);
}

/* VehFire_Audio without its cooldown (patched in by the test): the port leaves that to the listener. */
void TestFireAudio(int fireLevel)
{
	effect(" B %d", fireLevel);
}

/* PROC_CheckAllForDead for the turbo bucket. */
static void turbo_check_dead(void)
{
	if (gGT.threadBuckets[TURBO].thread != NULL && (turboThread.flags & THREAD_FLAG_DEAD) != 0)
	{
		turboThread.funcThDestroy(&turboThread);
		gGT.threadBuckets[TURBO].thread = NULL;
		memset(&turboObj, 0, sizeof(turboObj));
		memset(&turboInst1, 0, sizeof(turboInst1));
	}
}

void GhostTape_WriteBoosts(s32 addReserve, u32 type, s32 speedCap)
{
}

b32 LOAD_IsOpen_RacingOrBattle(void)
{
	return 1;
}

void VehStuckProc_MaskGrab_Init(struct Thread *t, struct Driver *drv)
{
	maskGrab = 1;
}

enum { S8, U8, S16, U16, S32, U32, QUAD, PHYS_STATE, INIT_PENDING, TERRAIN1, TERRAIN2, TURBO_EXISTS, TURBO_HIDDEN };

struct Field
{
	const char *name;
	void *p;
	int type;
};

#define F(member, type) { #member, &(member), type }
#define V3S16(v) F(v.x, S16), F(v.y, S16), F(v.z, S16)
#define V3S32(v) F(v.x, S32), F(v.y, S32), F(v.z, S32)
#define MAT(mat) \
	F(mat.m[0][0], S16), F(mat.m[0][1], S16), F(mat.m[0][2], S16), \
	F(mat.m[1][0], S16), F(mat.m[1][1], S16), F(mat.m[1][2], S16), \
	F(mat.m[2][0], S16), F(mat.m[2][1], S16), F(mat.m[2][2], S16)

static struct Field fields[] = {
	F(d.driverID, U8), F(d.kartState, U8), F(d.simpTurnState, S8),
	{ "phys_state", NULL, PHYS_STATE }, { "init_pending", NULL, INIT_PENDING },
	V3S32(d.posCurr), V3S32(d.posPrev), V3S32(d.velocity), V3S32(d.originToCenter),
	F(d.collisionFlags, S16), F(d.stepFlagSet, S32), F(d.actionsFlagSet, S32), F(d.actionsFlagSetPrevFrame, S32),
	F(d.currBlockTouching, QUAD), F(d.underDriver, QUAD), F(d.lastValid, QUAD),
	V3S16(d.normalVecUP), V3S16(d.AxisAngle1_normalVec), V3S16(d.AxisAngle2_normalVec),
	V3S16(d.AxisAngle3_normalVec), V3S16(d.AxisAngle4_normalVec),
	V3S16(d.spsHitPos), V3S16(d.spsNormalVec), V3S16(d.posWallColl),
	F(d.vShiftCount, S16), F(d.speed, S16), F(d.speedApprox, S16), F(d.baseSpeed, S16), F(d.fireSpeed, S16),
	F(d.fireSpeedCap, S16), F(d.jumpHeightCurr, S16), F(d.jumpHeightPrev, S16),
	F(d.xSpeed, S32), F(d.ySpeed, S32), F(d.zSpeed, S32), F(d.speedometerNeedleValue, S16),
	F(d.reserves, S16), F(d.turbo_outsideTimer, S16), F(d.turbo_MeterRoomLeft, S16),
	F(d.wallRubTimer, S16), F(d.wallRubSpeedLimit, S16),
	F(d.angle, S16), F(d.rotCurr.w, S16), F(d.turnAngleCurr, S16), F(d.turnAnglePrev, S16),
	F(d.turnAngleLerpVel, S16), F(d.turnAngleLerpTarget, S16),
	F(d.turnWobbleAngle, S16), F(d.turnWobbleVelocity, S16), F(d.turnWobbleTimer, S16), F(d.ampTurnState, S16),
	F(d.numFramesSpentSteering, S16), F(d.rotationSpinRate, S16), F(d.VehFire_AudioCooldown, S16),
	F(d.Screen_OffsetY, S8), F(d.distanceFromGround, S16), F(d.reserved_0x40e, S16),
	F(d.jumpSquishStretch, S16), F(d.jumpSquishStretch2, S16),
	F(inst.animIndex, U8), F(inst.animFrame, S16), F(inst.scale.x, S16), F(inst.scale.y, S16),
	F(d.matrixArray, U8), F(d.matrixIndex, U8),
	F(d.numWumpas, S8), F(d.accelConst, S8), F(d.turnConst, S8), F(d.turboConst, S8),
	F(d.clockReceive, S16), F(d.hazardTimer, S16), F(d.rainCloudEffect, S16),
	F(d.burnTimer, S16), F(d.squishTimer, S16), F(d.superEngineTimer, S16),
	F(d.accelTapWindowTimer, S16), F(d.accelTapCount, S16),
	F(d.vShiftStartGuardTimer, S16), F(d.vShiftWindowTimer, S16), F(d.driverRank, S16),
	F(d.jump_TenBuffer, S16), F(d.jump_CooldownMS, S16), F(d.jump_CoyoteTimerMS, S16), F(d.jump_ForcedMS, S16),
	F(d.jump_InitialVelY, S16), F(d.jump_HighJumpTimerMS, S16), F(d.jump_LandingBoost, S16),
	F(d.numberOfJumps, S16), F(d.jumpMeter, S16), F(d.jumpMeterTimer, S16), F(d.forcedJumpType, U8), F(d.buttonUsedToStartDrift, S16),
	F(d.rotCurr.x, S16), F(d.rotCurr.y, S16), F(d.rotCurr.z, S16),
	F(d.rotPrev.x, S16), F(d.rotPrev.y, S16), F(d.rotPrev.z, S16), F(d.rotPrev.w, S16),
	MAT(d.matrixMovingDir), MAT(d.matrixFacingDir),
	{ "terrainMeta1", NULL, TERRAIN1 }, { "terrainMeta2", NULL, TERRAIN2 },
	F(d.currentTerrain, U8), F(d.terrainFrictionTimer, S16), F(d.terrainScaledBaseSpeed, S16),
	F(d.normalVecID, S8), F(d.axisRotationX, S16), F(d.axisRotationY, S16), F(d.forwardDir, S16),
	V3S16(d.forwardAccelVector), F(d.forwardAccelImpulse, S16), V3S16(d.accel), F(d.wheelRotation, S16),
	F(d.boolFirstFrameSinceRevEngine, U8), F(d.quadBlockHeight, S32), F(d.distanceToFinish_checkpoint, U32),
	F(d.timeUntilDriftSpinout, S16), F(d.previousFrameMultDrift, S16),
	F(d.multDrift, S16), F(d.NoInputTimer, S16), F(d.failedBoostExhaustTimer, U8),
	F(d.KartStates.Drifting.numFramesDrifting, S16), F(d.KartStates.Drifting.driftBoostTimeMS, S16),
	F(d.KartStates.Drifting.driftTotalTimeMS, S16), F(d.KartStates.Drifting.numBoostsAttempted, S8),
	F(d.KartStates.Drifting.numBoostsSuccess, S8),
	{ "turbo", NULL, TURBO_EXISTS }, F(turboObj.fireAnimIndex, S16), F(turboObj.fireSize, S16),
	F(turboObj.fireDisappearCountdown, S8), F(turboObj.fireVisibilityCooldown, S16), F(turboInst1.alphaScale, U16),
	{ "turbo.hidden", NULL, TURBO_HIDDEN }, F(turboObj.fireAudioDistort, U8),
};

#define NUM_FIELDS ((int)(sizeof(fields) / sizeof(fields[0])))

static int terrainIndex(struct Terrain *t)
{
	return (int)(t - data.MetaDataTerrain);
}

static void set_state(int state, int initPending)
{
	memset(d.funcPtrs, 0, sizeof(d.funcPtrs));
	d.funcPtrs[DRIVER_FUNC_APPLY_FORCES] = VehPhysForce_OnApplyForces;
	d.funcPtrs[DRIVER_FUNC_COLL_MOVED] = COLL_MOVED_PlayerSearch;
	d.funcPtrs[DRIVER_FUNC_COLL_FIXED] = COLL_FIXED_PlayerSearch;
	d.funcPtrs[DRIVER_FUNC_COLLIDE_DRIVERS] = VehPhysForce_CollideDrivers;
	d.funcPtrs[DRIVER_FUNC_TRANSLATE_MATRIX] = VehPhysForce_TranslateMatrix;
	d.funcPtrs[DRIVER_FUNC_JUMP_FRICTION] = VehPhysGeneral_JumpAndFriction;
	d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_Driving_PhysLinear;
	d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysGeneral_PhysAngular;
	d.funcPtrs[DRIVER_FUNC_ANIMATE] = VehFrameProc_Driving;
	d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_Driving_Update;
	switch (state)
	{
	case 1:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_SlamWall_Update;
		d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_SlamWall_PhysLinear;
		d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysProc_SlamWall_PhysAngular;
		d.funcPtrs[DRIVER_FUNC_ANIMATE] = VehPhysProc_SlamWall_Animate;
		break;
	case 2:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_FreezeVShift_Update;
		d.funcPtrs[DRIVER_FUNC_JUMP_FRICTION] = VehPhysProc_FreezeVShift_ReverseOneFrame;
		break;
	case 3:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_PowerSlide_Update;
		d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_PowerSlide_PhysLinear;
		d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysProc_PowerSlide_PhysAngular;
		break;
	case 4:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_SpinFirst_Update;
		d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_SpinFirst_PhysLinear;
		d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysProc_SpinFirst_PhysAngular;
		d.funcPtrs[DRIVER_FUNC_ANIMATE] = VehFrameProc_Spinning;
		break;
	case 5:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_SpinLast_Update;
		d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_SpinLast_PhysLinear;
		d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysProc_SpinLast_PhysAngular;
		d.funcPtrs[DRIVER_FUNC_ANIMATE] = VehFrameProc_LastSpin;
		break;
	case 6:
		d.funcPtrs[DRIVER_FUNC_UPDATE] = VehPhysProc_SpinStop_Update;
		d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR] = VehPhysProc_SpinStop_PhysLinear;
		d.funcPtrs[DRIVER_FUNC_PHYS_ANGULAR] = VehPhysProc_SpinStop_PhysAngular;
		d.funcPtrs[DRIVER_FUNC_ANIMATE] = VehPhysProc_SpinStop_Animate;
		break;
	}
	if (initPending)
		d.funcPtrs[DRIVER_FUNC_INIT] = state == 3 ? VehPhysProc_PowerSlide_InitSetUpdate
		                              : state == 4 ? VehPhysProc_SpinFirst_InitSetUpdate
		                                           : VehPhysProc_Driving_Init;
}

static int current_state(void)
{
	DriverFunc linear = d.funcPtrs[DRIVER_FUNC_PHYS_LINEAR];
	if (linear == (DriverFunc)VehPhysProc_SlamWall_PhysLinear)
		return 1;
	if (linear == (DriverFunc)VehPhysProc_PowerSlide_PhysLinear)
		return 3;
	if (linear == (DriverFunc)VehPhysProc_SpinFirst_PhysLinear)
		return 4;
	if (linear == (DriverFunc)VehPhysProc_SpinLast_PhysLinear)
		return 5;
	if (linear == (DriverFunc)VehPhysProc_SpinStop_PhysLinear)
		return 6;
	return d.funcPtrs[DRIVER_FUNC_JUMP_FRICTION] == (DriverFunc)VehPhysProc_FreezeVShift_ReverseOneFrame ? 2 : 0;
}

static void load_driver(void)
{
	int state = 0, initPending = 0, turbo = 0, hidden = 0;
	gGT.threadBuckets[TURBO].thread = NULL;
	for (int i = 0; i < NUM_FIELDS; i++)
	{
		long long v = 0;
		void *p = fields[i].p;
		if (scanf("%lld", &v) != 1)
			exit(2);
		switch (fields[i].type)
		{
		case S8: *(s8 *)p = v; break;
		case U8: *(u8 *)p = v; break;
		case S16: *(s16 *)p = v; break;
		case U16: *(u16 *)p = v; break;
		case S32: *(s32 *)p = v; break;
		case U32: *(u32 *)p = v; break;
		case QUAD: *(struct QuadBlock **)p = v < 0 ? NULL : &quads[v]; break;
		case PHYS_STATE: state = v; break;
		case INIT_PENDING: initPending = v; break;
		case TERRAIN1: d.terrainMeta1 = &data.MetaDataTerrain[v]; break;
		case TERRAIN2: d.terrainMeta2 = &data.MetaDataTerrain[v]; break;
		case TURBO_EXISTS:
			turbo = v;
			if (turbo)
			{
				/* Born before the rest of its fields are read. */
				INSTANCE_BirthWithThread(STATIC_TURBO_EFFECT, NULL, 0, TURBO, VehTurbo_ThTick, sizeof(struct Turbo), NULL);
				turboThread.funcThDestroy = VehTurbo_ThDestroy;
				turboObj.driver = &d;
				turboObj.inst = INSTANCE_Birth3D(NULL, NULL, &turboThread);
			}
			break;
		case TURBO_HIDDEN: hidden = v; break;
		}
	}
	if (turbo)
	{
		turboInst1.flags = hidden ? HIDE_MODEL : 0;
		turboInst2.alphaScale = turboInst1.alphaScale;
	}
	else
	{
		memset(&turboObj, 0, sizeof(turboObj));
		memset(&turboInst1, 0, sizeof(turboInst1));
	}
	d.heldItemID = HELD_ITEM_NONE;
	set_state(state, initPending);
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
		case U32: printf(" %u", *(u32 *)p); break;
		case QUAD: printf(" %d", quadIndex(*(struct QuadBlock **)p)); break;
		case PHYS_STATE:
			printf(" %d", current_state());
			break;
		case INIT_PENDING: printf(" %d", d.funcPtrs[DRIVER_FUNC_INIT] != NULL); break;
		case TERRAIN1: printf(" %d", terrainIndex(d.terrainMeta1)); break;
		case TERRAIN2: printf(" %d", terrainIndex(d.terrainMeta2)); break;
		case TURBO_EXISTS: printf(" %d", gGT.threadBuckets[TURBO].thread != NULL); break;
		case TURBO_HIDDEN: printf(" %d", (turboInst1.flags & HIDE_MODEL) != 0); break;
		}
	}
}

/* The offsets and sizes of the constants `data.metaPhys` describes, in its order. */
static const int metaOffsets[65] = {GRAVITY_OFFSET, JUMP_OFFSET, PEDAL_FRICTION_PERPENDICULAR_OFFSET, PEDAL_FRICTION_FORWARD_OFFSET, NO_PEDAL_FRICTION_PERPENDICULAR_OFFSET, NO_PEDAL_FRICTION_FORWARD_OFFSET, BRAKE_FRICTION_OFFSET, DRIFT_CURVE_OFFSET, DRIFT_FRICTION_OFFSET, ACCEL_CLASS_STAT_OFFSET, ACCEL_RESERVES_OFFSET, SPEED_CLASS_STAT_OFFSET, ACCEL_SPEED_CLASS_STAT_OFFSET, SINGLE_TURBO_SPEED_OFFSET, SACRED_FIRE_SPEED_OFFSET, BACKWARD_SPEED_OFFSET, MASK_SPEED_OFFSET, DAMAGED_SPEED_OFFSET, TURN_RATE_OFFSET, BACKWARD_TURN_RATE_OFFSET, TURN_DECREASE_RATE_OFFSET, TURN_INPUT_DELAY_OFFSET, PRE_TURBO_OFFSET, TERMINAL_VELOCITY_OFFSET, TERRAIN_FRICTION_BOOST_OFFSET, STEER_ACCEL_STAGE4_FIRST_FRAME_OFFSET, STEER_ACCEL_STAGE2_FIRST_FRAME_OFFSET, STEER_ACCEL_STAGE2_FRAME_LENGTH_OFFSET, STEER_ACCEL_STAGE1_MAX_STEER_OFFSET, STEER_ACCEL_STAGE1_MIN_STEER_OFFSET, STEER_ACCEL_TURN_VEL_SCALE_OFFSET, STEER_ACCEL_TURN_VEL_LIMIT_OFFSET, MODEL_ROT_VEL_MAX_OFFSET, MODEL_ROT_VEL_MIN_OFFSET, MODEL_TURN_COUNTER_STEER_STRENGTH_OFFSET, MODEL_TURN_RETURN_STRENGTH_OFFSET, MODEL_TURN_NEGATIVE_RETURN_STRENGTH_OFFSET, MODEL_TURN_VELOCITY_LERP_OFFSET, TURN_RESIST_MIN_OFFSET, TURN_RESIST_MAX_OFFSET, STEER_VEL_DRIFT_SWITCH_WAY_OFFSET, STEER_VEL_DRIFT_STANDARD_OFFSET, DRIFT_TURN_BASE_OFFSET, DRIFT_TURN_STARTUP_SCALE_OFFSET, DRIFT_TURN_RAMP_FRAMES_OFFSET, DRIFT_FRAMES_TILL_SPINOUT_OFFSET, DRIFT_SPIN_RATE_ACCEL_OFFSET, DRIFT_SPIN_RATE_DECEL_OFFSET, DRIFT_CAMERA_SPIN_RATE_OFFSET, DRIFT_CAMERA_LERP_STEP_OFFSET, DRIFT_RELEASE_TURN_ASSIST_FRAMES_OFFSET, METAPHYS_33_OFFSET, METAPHYS_34_OFFSET, DRIFT_TURN_SAME_DIRECTION_ANGLE_OFFSET, DRIFT_TURN_OPPOSITE_DIRECTION_ANGLE_OFFSET, DRIFT_TURN_ANGLE_SCALE_OFFSET, TURBO_MAX_ROOM_OFFSET, TURBO_LOW_ROOM_WARNING_OFFSET, TURBO_FULL_BAR_RESERVE_GAIN_OFFSET, DRIFT_BOOST_DURATION_FRAMES_OFFSET, DRIFT_BOOST_AXIS_KICK_RATE_OFFSET, COLLISION_WEIGHT_OFFSET, SLOPE_FORWARD_SPEED_BONUS_OFFSET, SIDE_SPEED_CLAMP_OFFSET, PROTOTYPE_KEY_OFFSET};
static const int metaSizes[65] = {2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 2, 2, 2, 4};

int main(void)
{
	char cmd;

	memset(&ext, 0, sizeof(ext));
	for (int i = 0; i < 8; i++)
		data.characterIDs[i] = i;
	sdata_static.gGT = &gGT;
	sdata_static.gGamepads = &pads;
	gGT.level1 = &level;
	gGT.numPlyrCurrGame = 1;
	level.ptr_mesh_info = &mesh;
	/* The animations end where the real tables do; their contents only move the model's instance. */
	static const int bakedLengths[BAKED_GTE_MATRIX_COUNT] = {0, 0xb, 1, 9, 0x10, 0xf, 0x1b};
	for (int i = 0; i < BAKED_GTE_MATRIX_COUNT; i++)
	{
		data.bakedGteMath[i].physEntry = bakedDummy;
		data.bakedGteMath[i].numEntries = i < BAKED_GTE_MATRIX_JUMP_BASE ? bakedLengths[i] : 4;
	}
	thread.inst = &inst;
	thread.modelIndex = DYNAMIC_PLAYER;
	inst.thread = &thread;
	d.instSelf = &inst;
	inst.model = &model;
	model.numHeaders = 1;
	model.headers = &modelHeader;
	modelHeader.numAnimations = 8;
	modelHeader.ptrAnimations = modelAnims;
	for (int i = 0; i < 8; i++)
		modelAnims[i] = &modelAnimData[i];

	while (scanf(" %c", &cmd) == 1)
	{
		if (read_level_block(cmd))
			continue;
		switch (cmd)
		{
		case 'E':
			for (int i = 0; i < numQuads; i++)
			{
				quads[i].mulNormVecY = rd();
				quads[i].checkpointIndex = rd();
			}
			level.jumpVerticalSpeedCap = rd();
			level.configFlags = rd();
			{
				int n = rd();
				restartPoints = calloc(n ? n : 1, sizeof(*restartPoints));
				for (int i = 0; i < n; i++)
				{
					restartPoints[i].distToFinish = rd();
					restartPoints[i].nextIndex_forward = rd();
				}
				level.ptr_restart_points = restartPoints;
			}
			break;
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
		case 'R':
			for (int i = 0; i < 21; i++)
			{
				struct Terrain *t = &data.MetaDataTerrain[i];
				t->flags = rd();
				t->speedMultiplier = rd();
				t->slowUntilSpeed = rd();
				t->counterSteerRatio = rd();
				t->turnLeanScale = rd();
				t->groundFrictionScale = rd();
				t->turnAngleScale = rd();
				t->turnResponseScale = rd();
				t->sound = rd();
			}
			break;
		case 'A':
			for (int i = 0; i < 8; i++)
				modelAnimData[i].numFrames = rd();
			break;
		case 'S':
			for (int i = 0; i < 65; i++)
			{
				int v = rd();
				u8 *p = (u8 *)&d + metaOffsets[i];
				if (metaSizes[i] == 1)
					*p = v;
				else if (metaSizes[i] == 2)
					*(s16 *)p = v;
				else
					*(s32 *)p = v;
			}
			break;
		case 'C':
		{
			int character = rd();
			for (int i = 0; i < 8; i++)
				data.characterIDs[i] = character;
			break;
		}
		case 'N':
			for (int i = 0; i < NUM_FIELDS; i++)
				printf("%s%s", i ? " " : "", fields[i].name);
			printf("\n");
			break;
		case 'D':
			load_driver();
			break;
		case 'I':
		{
			int reserves = rd(), type = rd(), fireLevel = rd();
			VehFire_Increment(&d, reserves, type, fireLevel);
			break;
		}
		case 'X':
		{
			gGT.elapsedTimeMS = rd();
			gGT.elapsedEventTime = rd();
			gGT.timer = rd();
			gGT.gameMode1 = rd() ? END_OF_RACE : 0;
			pads.gamepad[d.driverID].buttonsHeldCurrFrame = rd();
			pads.gamepad[d.driverID].buttonsTapped = rd();
			pads.gamepad[d.driverID].stickLX = rd();
			pads.gamepad[d.driverID].stickLY = rd();
			pads.gamepad[d.driverID].stickRY = rd();

			/* (An 'I' before this tick leaves its effects in the line.) */
			maskGrab = 0;
			for (int i = 0; i < DRIVER_FUNC_COUNT; i++)
			{
				/* Audio and particles are not ported. */
				if (i == DRIVER_FUNC_AUDIO || i == DRIVER_FUNC_PARTICLES)
					continue;
				DriverFunc fn = d.funcPtrs[i];
				if (fn == NULL)
					continue;
				/* Driving_Init installs the real one again. */
				if (i == DRIVER_FUNC_TRANSLATE_MATRIX)
					fn = VehPhysForce_TranslateMatrix;
				fn(&thread, &d);
			}
			/* The buckets after the drivers': the turbo fire. */
			if (gGT.threadBuckets[TURBO].thread != NULL)
			{
				/* Its GTE work only places the fire's instances; the port's fire does none, so
				 * the GTE goes on to the next tick as the drivers' stages left it. */
				GTERegisters gte = gteRegs;
				inTurboTick = 1;
				VehTurbo_ThTick(&turboThread);
				inTurboTick = 0;
				gteRegs = gte;
			}
			turbo_check_dead();
			/* The HUD's jump meter gives the hang-time turbo, in the frame's render after all that. */
			UI_JumpMeter_Update(&d);

			struct ScratchpadStruct *pad = (struct ScratchpadStruct *)(scratchpad + 0x108);
			print_driver();
			printf(" |%s | %d %d %u %d %d %d", effects, pad->hitFraction, pad->Input1.hitRadius, pad->collision.stepFlags,
			       pad->boolDidTouchQuadblock, pad->numTrianglesTested, maskGrab);
			printf("\n");
			effects[0] = 0;
			break;
		}
		}
	}
	return 0;
}
