(ns jepsen.openraft.liveness-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.openraft.liveness :as liveness]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(defn event [f time status]
  {:process :nemesis :type :info :f f :time time :value {:status status}})

(defn writes [start finish]
  [{:process 0 :type :invoke :f :write :time start :value ["k" "v"]}
   {:process 0 :type :ok :f :write :time finish :value ["k" "v"]}])

(def base-history
  [(update (event :start-liveness 0 :installed) :value assoc
           :majority ["n1" "n2" "n3"] :leader "n4" :isolated "n5")
   (assoc-in (event :sample-liveness 100000000 :observed)
             [:value :metrics]
             (zipmap nodes
                     (repeat {:membership_config {:membership {:configs [nodes]}}})))
   {:process :nemesis :type :info :f :stop-liveness :time 10000000000}
   (event :stop-liveness 11000000000 :recovered)])

(defn verdict [history]
  (checker/check (liveness/liveness-checker) {:nodes nodes}
                 (sort-by :time history) {}))

(def passing-history
  (concat base-history (writes 100000000 200000000)))

(deftest topology-test
  (is (= {:leader "n4" :bridge "n1" :majority ["n1" "n2" "n3"]
          :cut ["n2" "n3"] :isolated "n5"}
         (liveness/topology nodes "n4")))
  (doseq [leader nodes]
    (let [{:keys [majority bridge cut isolated] :as plan} (liveness/topology nodes leader)]
      (is (= leader (:leader plan)))
      (is (= (frequencies nodes) (frequencies (concat majority [leader isolated]))))
      (is (= 3 (count majority)))
      (is (some #{bridge} majority))
      (is (= (set (remove #{bridge} majority)) (set cut)))))
  (doseq [invalid [(vec (butlast nodes)) (conj nodes "n6")
                   ["n1" "n2" "n3" "n4" "n4"]]]
    (is (thrown? clojure.lang.ExceptionInfo
                 (liveness/topology invalid "n1"))))
  (is (thrown? clojure.lang.ExceptionInfo
               (liveness/topology nodes "n6"))))

(deftest phase-boundaries-test
  (testing "writes during and after recovery cannot hide the fault-period stall"
    (let [result (verdict (concat base-history
                                  (writes 10100000000 10200000000)
                                  (writes 12000000000 12100000000)))]
      (is (false? (:valid? result)))
      (is (zero? (get-in result [:original-topology :successful-writes])))))
  (testing "timely writes pass without leader stability evidence"
    (is (true? (:valid? (verdict passing-history)))))
  (testing "an invocation before installation does not count"
    (is (zero? (get-in (verdict (concat base-history (writes -1 100000000)))
                       [:original-topology :successful-writes]))))
  (testing "a four-voter membership cannot pass the five-node scenario"
    (let [result (verdict
                  (map #(if (= :sample-liveness (:f %))
                          (update-in % [:value :metrics]
                                     (fn [metrics]
                                       (into {} (for [[node m] metrics]
                                                  [node (assoc-in m [:membership_config :membership :configs]
                                                                  [(vec (butlast nodes))])]))))
                          %)
                       passing-history))]
      (is (false? (:five-voters? result)))
      (is (false? (:valid? result)))))
  (testing "missing old-leader or isolated-voter observations cannot pass"
    (doseq [node ["n4" "n5"]]
      (is (false?
           (:valid?
            (verdict
             (map #(if (= :sample-liveness (:f %))
                     (update-in % [:value :metrics] dissoc node) %)
                  passing-history)))) node)))
  (testing "success after the deadline is reported but fails the bound"
    (let [result (verdict (concat base-history
                                  (writes 1600000000 1700000000)))]
      (is (= 1 (get-in result [:original-topology :successful-writes])))
      (is (false? (get-in result [:original-topology :within-deadline?])))
      (is (false? (:valid? result)))))
  (testing "missing recovery alone prevents success"
    (is (false? (:valid? (verdict (remove #(= :recovered (get-in % [:value :status]))
                                          passing-history))))))
  (testing "empty history fails closed"
    (is (false? (:valid? (verdict []))))))
