(ns jepsen.openraft.pre-vote-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [control :as c]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.pre-vote :as pre-vote]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])
(def second-nanos 1000000000)

(def metrics
  (into {} (for [node nodes]
             [node {:current_term 7
                    :state (if (= "n1" node) "Leader" "Follower")
                    :last_applied {:leader_id {:term 7 :node_id "n1"} :index 9}
                    :last_log_index 9
                    :membership_config {:membership {:configs [nodes]}}}])))

(def activity
  (into {} (for [node nodes]
             [node {:leader-exits 0
                    :leader-entries (if (= "n1" node) 1 0)}])))

(defn event [f time value]
  {:process :nemesis :type :info :f f :time time :value value})

(def base-history
  [(event :start-pre-vote 0
          (assoc (pre-vote/topology nodes)
                 :status :installed :metrics metrics
                 :baseline {:leader "n1" :metrics metrics :activity activity}))
   (event :sample-pre-vote second-nanos {:status :observed :metrics metrics})
   (event :sample-pre-vote (* 10 second-nanos) {:status :observed :metrics metrics})
   (event :sample-pre-vote (* 19 second-nanos) {:status :observed :metrics metrics})
   (event :stop-pre-vote (* 20 second-nanos) nil)
   (event :stop-pre-vote (* 21 second-nanos)
          {:status :recovered
           :before-heal {:metrics metrics}
           :recovery-metrics [metrics]})
   (assoc (event :sample-pre-vote (* 22 second-nanos)
                 {:status :observed :metrics metrics :activity activity})
          :final-sample? true)])

(defn writes [start finish]
  [{:process 0 :type :invoke :f :write :time start :value ["k" "v"]}
   {:process 0 :type :ok :f :write :time finish :value ["k" "v"]}])

(def workload
  (mapcat #(writes (+ (* % second-nanos) 400000000)
                   (+ (* % second-nanos) 500000000))
          (range 20)))

(def passing-history (concat base-history workload))

(defn verdict [history]
  (checker/check (pre-vote/stability-checker) {:nodes nodes}
                 (sort-by :time history) {}))

(defn change-event [history predicate f]
  (map #(if (predicate %) (f %) %) history))

(deftest topology-retains-exactly-the-requested-links
  (let [{:keys [retained blocked] :as plan} (pre-vote/topology nodes)
        retained (set (map set retained))
        blocked (set (for [[node peers] blocked peer peers] #{node peer}))
        all-pairs (set (for [a nodes b nodes :when (neg? (compare a b))] #{a b}))]
    (is (= "n1" (:leader plan)))
    (is (= ["n1" "n2" "n3"] (:core plan)))
    (is (= "n4" (:disconnected plan)))
    (is (= #{#{"n1" "n2"} #{"n1" "n3"} #{"n2" "n3"}
             #{"n2" "n4"} #{"n1" "n5"}} retained))
    (is (= all-pairs (into retained blocked)))
    (is (not-any? retained blocked)))
  (doseq [invalid [(vec (butlast nodes)) (conj nodes "n6")
                   ["n1" "n2" "n3" "n4" "n4"]]]
    (is (thrown? clojure.lang.ExceptionInfo (pre-vote/topology invalid)))))

(deftest stability-checker-accepts-a-complete-stable-execution
  (let [result (verdict passing-history)]
    (is (true? (:valid? result)))
    (is (= 7 (:term result)))
    (is (= "n1" (:leader result)))
    (is (= 20 (:successful-writes result)))
    (is (= 1000.0 (:max-write-gap-ms result)))))

(deftest term-and-role-changes-fail-in-every-phase
  (doseq [path [[:value :baseline :metrics]
                [:value :metrics]
                [:value :before-heal :metrics]
                [:value :recovery-metrics 0]]
          node nodes]
    (testing (str "term bump at " path " on " node)
      (let [history (change-event passing-history #(some? (get-in % path))
                                  #(assoc-in % (into path [node :current_term]) 8))]
        (is (false? (:valid? (verdict history)))))))
  (testing "a transient step-down observed at one sample is retained"
    (let [result (verdict
                  (change-event passing-history #(= second-nanos (:time %))
                                #(assoc-in % [:value :metrics "n1" :state] "Follower")))]
      (is (false? (:valid? result)))
      (is (= [{:node "n1" :state "Follower"}] (:role-errors result)))))
  (testing "a leader transition between polls appears in the log counters"
    (doseq [[node counter] [["n1" :leader-exits] ["n1" :leader-entries]
                            ["n2" :leader-entries]]]
      (let [result (verdict
                    (change-event passing-history :final-sample?
                                  #(update-in % [:value :activity node counter] inc)))]
        (is (false? (:leadership-stable? result)))
        (is (false? (:valid? result)))))))

(deftest checker-requires-complete-evidence
  (testing "missing node metrics fail in any observed phase"
    (doseq [path [[:value :baseline :metrics] [:value :metrics]
                  [:value :before-heal :metrics] [:value :recovery-metrics 0]]
            node nodes]
      (let [result (verdict
                    (change-event passing-history #(some? (get-in % path))
                                  #(update-in % path dissoc node)))]
        (is (false? (:valid? result)))
        (is (false? (:complete-observations? result))))))
  (testing "missing mandatory events cannot pass"
    (doseq [predicate [#(= :start-pre-vote (:f %))
                       #(and (= :sample-pre-vote (:f %)) (not (:final-sample? %)))
                       :final-sample?
                       #(= :recovered (get-in % [:value :status]))]]
      (is (false? (:valid? (verdict (remove predicate passing-history)))))))
  (doseq [[time path value]
          [[(* 21 second-nanos) [:value :recovery-metrics] []]
           [(* 21 second-nanos) [:value :recovery-metrics 0 "n4" :last_log_index] 8]
           [0 [:value :baseline :metrics "n1" :current_term] nil]
           [second-nanos [:value :metrics "n4" :membership_config :membership :configs]
            [(vec (butlast nodes))]]
           [0 [:value :retained] []]
           [0 [:value :leader] "n3"]]]
    (testing (str "invalid evidence at " time ": " path)
      (is (false? (:valid? (verdict (change-event passing-history #(= time (:time %))
                                                  #(assoc-in % path value))))))))
  (testing "samples only after healing do not establish partition coverage"
    (is (false? (:valid? (verdict
                          (change-event passing-history
                                        #(and (= :sample-pre-vote (:f %)) (not (:final-sample? %)))
                                        #(assoc % :time (+ (* 21 second-nanos) 100000000))))))))
  (testing "empty history fails closed"
    (is (false? (:valid? (verdict []))))))

(deftest progress-is-required-throughout-the-full-partition
  (testing "early, middle, and tail stalls fail despite other successful writes"
    (doseq [[begin end] [[0 2] [9 11] [18 20]]]
      (let [result (verdict
                    (concat base-history
                            (remove #(<= (* begin second-nanos) (:time %) (* end second-nanos))
                                    workload)))]
        (is (false? (:valid? result)))
        (is (> (:max-write-gap-ms result) 1500.0)))))
  (testing "writes invoked before installation and writes during healing do not count"
    (let [result (verdict
                  (concat base-history
                          (writes -1 500000000)
                          (writes (* 20 second-nanos) (+ (* 20 second-nanos) 100000000))
                          (writes (* 22 second-nanos) (+ (* 22 second-nanos) 100000000))))]
      (is (zero? (:successful-writes result)))
      (is (false? (:valid? result)))))
  (testing "a write completing exactly at healing starts is outside the fault interval"
    (let [result (verdict
                  (concat base-history
                          (writes (- (* 20 second-nanos) 100000000) (* 20 second-nanos))))]
      (is (zero? (:successful-writes result)))))
  (testing "an incomplete twenty-second observation cannot pass"
    (let [result (verdict
                  (change-event passing-history #(= (* 20 second-nanos) (:time %))
                                #(update % :time dec)))]
      (is (false? (:observation-complete? result)))
      (is (false? (:valid? result))))))

(defn- with-firewall [f]
  (let [commands (atom [])
        current-node (atom nil)
        roles (atom nil)
        plan (pre-vote/topology nodes)
        addresses (zipmap nodes ["127.0.0.1" "127.0.0.2" "127.0.0.3" "127.0.0.4" "127.0.0.5"])
        test {:nodes nodes :raft-port 22001
              :bootstrap-state (atom {:leader "n1" :metrics metrics})}
        subject (:nemesis (pre-vote/package roles))
        on-nodes (fn on-nodes
                   ([test f] (on-nodes test (:nodes test) f))
                   ([test targets f]
                    (into {} (for [node targets]
                               (do (reset! current-node node)
                                   [node (f test node)])))))]
    (with-redefs [c/on-nodes on-nodes
                  c/exec (fn [& args]
                           (swap! commands conj [@current-node (vec args)])
                           (cond
                             (= :awk (first args)) "1 0\n"
                             (= [:iptables :-S] (vec args))
                             (if (= "n4" @current-node)
                               "-N OR_PREVOTE\n-A INPUT -j OR_PREVOTE\n-A OUTPUT -j OR_PREVOTE\n"
                               "-P INPUT ACCEPT\n-P OUTPUT ACCEPT\n")
                             :else ""))
                  http/node-host addresses
                  cluster/node-metrics! (fn [_ node] (get metrics node))]
      (f {:commands commands :roles roles :plan plan :addresses addresses
          :test test :subject subject}))))

(deftest firewall-installs-and-verifies-only-the-raft-cuts
  (with-firewall
    (fn [{:keys [commands roles plan addresses test subject]}]
      (let [result (nemesis/invoke! subject test {:type :info :f :start-pre-vote})
            additions (filter #(= [:iptables :-A] (take 2 (second %))) @commands)
            expected (set (for [[node peers] (:blocked plan) peer peers direction [:-s :-d]]
                            [node [:iptables :-A :OR_PREVOTE direction (get addresses peer)
                                   :-p :tcp :-m :multiport :--ports 22001 :-j :DROP]]))]
        (is (= :installed (get-in result [:value :status])))
        (is (= plan @roles))
        (is (= expected (set additions)))
        (doseq [[node [_ _ & rule]] additions]
          (is (some #{[node (into [:iptables :-C] rule)]} @commands)))
        (doseq [node ["n4" "n5"] chain [:INPUT :OUTPUT]]
          (is (some #{[node [:iptables :-I chain :-j :OR_PREVOTE]]} @commands))
          (is (some #{[node [:iptables :-C chain :-j :OR_PREVOTE]]} @commands)))))))

(deftest failed-installation-keeps-the-plan-for-cleanup
  (doseq [failure [:-A :-C]]
    (with-firewall
      (fn [{:keys [commands roles plan test subject]}]
        (with-redefs [c/exec (fn [& args]
                               (cond
                                 (= :awk (first args)) "1 0\n"
                                 (= [:iptables failure] (take 2 args))
                                 (throw (ex-info "iptables failed" {}))
                                 :else ""))]
          (is (thrown-with-msg? clojure.lang.ExceptionInfo #"iptables failed"
                                (nemesis/invoke! subject test {:type :info :f :start-pre-vote}))))
        (is (= plan @roles))
        (nemesis/teardown! subject test)
        (is (= [["n4" [:iptables :-S]]
                ["n4" [:iptables :-D :INPUT :-j :OR_PREVOTE]]
                ["n4" [:iptables :-D :OUTPUT :-j :OR_PREVOTE]]
                ["n4" [:iptables :-F :OR_PREVOTE]]
                ["n4" [:iptables :-X :OR_PREVOTE]]
                ["n5" [:iptables :-S]]]
               @commands))))))

(deftest failed-rules-listing-is-not-successful-cleanup
  (with-firewall
    (fn [{:keys [roles plan test subject]}]
      (reset! roles plan)
      (with-redefs [c/exec (fn [& _] (throw (ex-info "iptables listing failed" {})))]
        (is (thrown-with-msg? clojure.lang.ExceptionInfo #"iptables listing failed"
                              (nemesis/teardown! subject test)))))))

(deftest diagnostic-failure-still-heals-the-partition
  (with-firewall
    (fn [{:keys [commands roles plan test subject]}]
      (reset! roles plan)
      (with-redefs [cluster/node-metrics! (fn [& _] (throw (ex-info "metrics failed" {})))]
        (is (thrown-with-msg? clojure.lang.ExceptionInfo #"metrics failed"
                              (nemesis/invoke! subject test {:type :info :f :stop-pre-vote}))))
      (is (some #{["n4" [:iptables :-X :OR_PREVOTE]]} @commands))
      (is (some #{["n5" [:iptables :-S]]} @commands)))))

(deftest cleanup-failure-preserves-the-diagnostic-error
  (with-firewall
    (fn [{:keys [roles plan test subject]}]
      (reset! roles plan)
      (let [error (ex-info "metrics failed" {})
            cleanup (ex-info "cleanup failed" {})]
        (with-redefs [cluster/node-metrics! (fn [& _] (throw error))
                      c/exec (fn [& _] (throw cleanup))]
          (is (identical? error (try
                                  (nemesis/invoke! subject test {:type :info :f :stop-pre-vote})
                                  (catch Exception e e))))
          (is (= [cleanup] (vec (.getSuppressed error)))))))))

(deftest generator-observes-for-twenty-seconds-before-healing
  (doseq [latency [0 100000000]]
    (testing (str "operation latency=" latency)
      (with-redefs [gen-test/rand-seed 0]
        (let [workload {:generator (gen/clients (gen/stagger 0.1 (repeat {:f :write})))
                        :final-generator (gen/clients [{:f :final-read}])}
              history (vec
                       (gen-test/simulate
                        (pre-vote/generator (harness/failure-state) workload)
                        (fn [_ operation]
                          (cond-> (-> operation
                                      (assoc :type (if (= :nemesis (:process operation)) :info :ok))
                                      (update :time + (if (= :sleep (:type operation))
                                                        (long (* second-nanos (:value operation)))
                                                        latency)))
                            (= :start-pre-vote (:f operation))
                            (assoc :value {:status :installed})))))
              start (first (filter #(= :installed (get-in % [:value :status])) history))
              stop (first (filter #(= :stop-pre-vote (:f %)) history))
              final-read (first (filter #(= :final-read (:f %)) history))
              final-sample (first (filter :final-sample? history))]
          (is (<= (* 20 second-nanos) (- (:time stop) (:time start))))
          (is (< (.indexOf history stop) (.indexOf history final-read) (.indexOf history final-sample)))
          (is (not-any? :final? (filter #(= :nemesis (:process %)) history)))
          (is (some #(and (= :write (:f %)) (< (:time start) (:time %) (:time stop))) history)))))))

(deftest generator-heals-after-a-harness-failure
  (let [failure-state (harness/failure-state)
        operations (atom [])
        workload {:generator (gen/stagger 0.1 (repeat {:f :write}))
                  :final-generator [{:f :final-read}]}
        history (gen-test/simulate
                 (pre-vote/generator failure-state workload)
                 (fn [_ operation]
                   (swap! operations conj (:f operation))
                   (when (= :write (:f operation))
                     (harness/record-failure! failure-state :client {:operation operation}
                                              (RuntimeException. "client failed")))
                   (-> operation
                       (assoc :type (if (= :nemesis (:process operation)) :info :ok))
                       (update :time + 10))))]
    (is (seq history))
    (is (some? (harness/primary-failure failure-state)))
    (is (= 1 (count (filter #{:write} @operations))))
    (is (= :start-pre-vote (first @operations)))
    (is (= :stop-pre-vote (last @operations)))
    (is (not-any? #{:final-read} @operations))))
