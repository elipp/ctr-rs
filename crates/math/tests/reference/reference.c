/*
 * Test driver for the Rust math port: links the reference ctr-native math code
 * and executes a command script from stdin; each command prints its result.
 *
 *   s <angle>      MATH_Sin
 *   c <angle>      MATH_Cos
 *   S <a>          rsin
 *   C <a>          rcos
 *   a <y> <x>      ratan2
 *   q <v>          SquareRoot0
 *   Q <v>          SquareRoot0_stub
 *   f <v> <shift>  MATH_FastSqrt
 *   l <x> <y> <z>  MATH_VectorLength
 *   n <x> <y> <z>  MATH_VectorNormalize, prints length and the new vector
 */
#include <common.h>
#include <stdio.h>
#include <string.h>

/*
 * The real `data` segment pulls in most of the game, so only the trig table is
 * taken from the reference: the test extracts it from game/zGlobal_DATA.c into
 * TRIG_APPROX_INC (the contents of the `.trigApprox = { ... }` initialiser).
 */
struct Data data;
struct sData sdata_static;

static const struct TrigTable trig_init[0x400] = {
#include TRIG_APPROX_INC
};

__attribute__((constructor)) static void init_trig(void)
{
	memcpy(data.trigApprox, trig_init, sizeof(trig_init));
}

int rsin(int a);
int rcos(int a);
int ratan2(int y, int x);
int SquareRoot0(int a);
s32 SquareRoot0_stub(s32 value);

int main(void)
{
	char cmd;
	int a, b, c;

	while (scanf(" %c", &cmd) == 1)
	{
		switch (cmd)
		{
		case 's':
			scanf("%d", &a);
			printf("%d\n", MATH_Sin((u32)a));
			break;
		case 'c':
			scanf("%d", &a);
			printf("%d\n", MATH_Cos((u32)a));
			break;
		case 'S':
			scanf("%d", &a);
			printf("%d\n", rsin(a));
			break;
		case 'C':
			scanf("%d", &a);
			printf("%d\n", rcos(a));
			break;
		case 'a':
			scanf("%d %d", &a, &b);
			printf("%d\n", ratan2(a, b));
			break;
		case 'q':
			scanf("%d", &a);
			printf("%d\n", SquareRoot0(a));
			break;
		case 'Q':
			scanf("%d", &a);
			printf("%d\n", SquareRoot0_stub(a));
			break;
		case 'f':
			scanf("%d %d", &a, &b);
			printf("%d\n", MATH_FastSqrt((u32)a, b));
			break;
		case 'l':
		{
			SVec3 v;
			scanf("%d %d %d", &a, &b, &c);
			v.x = a, v.y = b, v.z = c;
			printf("%d\n", MATH_VectorLength(&v));
			break;
		}
		case 'n':
		{
			SVec3 v;
			scanf("%d %d %d", &a, &b, &c);
			v.x = a, v.y = b, v.z = c;
			int len = MATH_VectorNormalize(&v);
			printf("%d %d %d %d\n", len, v.x, v.y, v.z);
			break;
		}
		}
	}
	return 0;
}
