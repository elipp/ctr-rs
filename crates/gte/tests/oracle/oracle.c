/*
 * Test oracle for the Rust GTE port: links the reference ctr-native GTE and
 * executes a command script from stdin.
 *
 *   m <reg> <val>   MTC2
 *   c <reg> <val>   CTC2
 *   r <reg>         MFC2, prints the value
 *   x <op>          COP2 command, prints the result and all 64 registers
 *
 * Build (32-bit, like the reference):
 *   gcc -m32 -DCTR_NATIVE -I <ref>/include oracle.c <ref>/platform/native_gte_core.c \
 *       <ref>/platform/native_inline_c.c
 */
#include <stdio.h>
#include <psx/gtereg.h>
#include <psx/inline_c.h>

int main(void)
{
	char cmd;
	unsigned a, b;

	while (scanf(" %c", &cmd) == 1)
	{
		switch (cmd)
		{
		case 'm':
			scanf("%u %u", &a, &b);
			MTC2(b, a);
			break;
		case 'c':
			scanf("%u %u", &a, &b);
			CTC2(b, a);
			break;
		case 'r':
			scanf("%u", &a);
			printf("%u\n", MFC2(a));
			break;
		case 'x':
		{
			scanf("%u", &a);
			int ret = doCOP2(a);
			printf("%d", ret);
			for (int i = 0; i < 32; i++)
				printf(" %x", gteRegs.CP2D.r[i]);
			for (int i = 0; i < 32; i++)
				printf(" %x", gteRegs.CP2C.r[i]);
			printf("\n");
			break;
		}
		}
	}
	return 0;
}
