public class Objects {
    record Point(long x, long y) {}
    static long manhattan(Point p) { return Math.abs(p.x()) + Math.abs(p.y()); }
    public static void main(String[] args) {
        long sum = 0;
        for (long i = 0; i < 10000000L; i++) {
            Point p = new Point(i % 100 - 50, i % 37 - 18);
            sum += manhattan(p);
        }
        System.out.println(sum);
    }
}
