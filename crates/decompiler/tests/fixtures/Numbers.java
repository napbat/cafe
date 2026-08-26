package differential;

/** Deterministic arithmetic, conversion, and comparison surfaces. */
public final class Numbers {
    private Numbers() {}

    public static int add(int a, int b) {
        return a + b;
    }

    public static int divide(int a, int b) {
        return a / b;
    }

    public static int remainder(int a, int b) {
        return a % b;
    }

    public static int shifts(int a, int b) {
        return (a << b) ^ (a >> b) ^ (a >>> b);
    }

    public static int negate(int a) {
        return -a;
    }

    public static int bitwise(int a, int b) {
        return (a & b) | (a ^ ~b);
    }

    public static long longMath(long a, long b) {
        return a * 31L - (a ^ b) + (a >>> 7);
    }

    public static int longOrder(long a, long b) {
        return a < b ? -1 : (a > b ? 1 : 0);
    }

    public static int floatLess(float a, float b) {
        return a < b ? 10 : 20;
    }

    public static int floatGreater(float a, float b) {
        return a > b ? 10 : 20;
    }

    public static int doubleAtMost(double a, double b) {
        if (a <= b) {
            return -5;
        }
        return 5;
    }

    public static float floatMath(float a, float b) {
        return a * b + a / b;
    }

    public static double mixed(int a, long b, double c) {
        return a + b * c;
    }

    public static int truncations(int a) {
        return (byte) a + (short) a + (char) a;
    }

    public static long widen(int a) {
        return (long) a * 1000000007L;
    }

    public static float bitsToFloat(int a) {
        return Float.intBitsToFloat(a);
    }

    public static boolean isNan(double a) {
        return a != a;
    }

    public static int increments(int a) {
        int total = a;
        total += 3;
        total *= 2;
        total--;
        return total;
    }

    public static int half(int a) {
        return a / 2;
    }

    public static int viaHelper(int a) {
        int start = half(a) + 1;
        return half(start) + fibonacci(3);
    }

    public static int fibonacci(int n) {
        if (n < 2) {
            return n;
        }
        return fibonacci(n - 1) + fibonacci(n - 2);
    }
}
