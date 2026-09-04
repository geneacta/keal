public class Loops {
    public static void main(String[] args) {
        long total = 0;
        long i = 0;
        while (i < 100000000L) { total += i % 7; i += 1; }
        System.out.println(total);
    }
}
