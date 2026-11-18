// A subclass declaring a field with its superclass's name and descriptor has TWO fields: each
// class's code reaches its own, and a field access through a reference of the superclass's type
// reaches the superclass's (JVMS 5.4.3.2 - resolution starts at the class the instruction names).
class FieldShadow {
    static class Base {
        private boolean e;
        public byte d;

        Base() {
            e = true;
            d = 7;
        }

        void clear() {
            e = false;
        }

        boolean baseE() {
            return e;
        }

        int baseD() {
            return d;
        }
    }

    static class Derived extends Base {
        private boolean e;
        public byte d;

        Derived() {
            super();
            e = true;
            d = 42;
        }

        boolean derivedE() {
            return e;
        }

        int derivedD() {
            return d;
        }
    }

    public static void main(String[] args) {
        Derived x = new Derived();
        x.clear();
        System.out.println(x.baseE());
        System.out.println(x.derivedE());
        System.out.println(x.baseD());
        System.out.println(x.derivedD());

        Base b = x;
        b.d = 9;
        System.out.println(b.d);
        System.out.println(x.d);
        System.out.println(x.baseD());
    }
}
