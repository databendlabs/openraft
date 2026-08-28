(ns jepsen.openraft.nemesis.membership-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft [await :as await]
             [client :as client]
             [cluster :as cluster]
             [db :as openraft-db]
             [harness :as harness]
             [worker :as worker]]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.random :as random]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(def test-config
  {:nodes nodes
   :api-port 21001
   :raft-port 22001})

(defn- installed [details]
  (assoc details :status :installed))

(defn- skipped [reason details]
  (assoc details :status :skipped :reason reason))

(defn- indeterminate [reason details]
  (assoc details :status :indeterminate :reason reason))

(defn- condition-timeout [condition]
  (try
    (await/until! condition
                  #(await/retry! condition {})
                  {:timeout 0
                   :retry-interval 0})
    nil
    (catch Exception e
      e)))

(defn- membership-nemesis []
  (membership/->MembershipNemesis :database (atom nil)))

(defn- stable-status [voters]
  {:leader "n1"
   :voters voters
   :learners #{}
   :non-members (set (remove voters nodes))
   :stable? true
   :metrics (zipmap voters (repeat {}))})

(deftest grows-without-waiting-for-stable-membership
  (let [calls (atom [])
        before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        statuses (atom [(stable-status before)
                        (update (stable-status before)
                                :metrics
                                assoc
                                "n5"
                                {})])]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime grow must not wait" {})))
                  openraft-db/start-empty-node-without-wait!
                  (fn [database _test node]
                    (swap! calls conj [:start-empty database node]))
                  client/add-learner!
                  (fn [endpoint node-id api-addr raft-addr]
                    (swap! calls conj
                           [:add-learner endpoint node-id api-addr raft-addr]))
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))]
      (let [subject (membership-nemesis)
            starting (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :grow})
            result (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= :skipped (get-in starting [:value :status])))
        (is (= :node-starting (get-in starting [:value :reason])))
        (is (= [[:start-empty :database "n5"]
                [:add-learner
                 "n1:21001"
                 "n5"
                 "n5:21001"
                 "n5:22001"]
                [:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4" "n5"]]]
               @calls))
        (is (= {:change :grow
                :node "n5"
                :source :non-member
                :before before
                :after after}
               (select-keys (:value result)
                            [:change :node :source :before :after])))
        (is (= :installed (get-in result [:value :status])))))))

(deftest handles-an-existing-learner
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        calls (atom [])
        reachable (assoc (stable-status before)
                         :learners #{"n5"}
                         :non-members #{}
                         :metrics {"n1" {} "n2" {} "n3" {}
                                   "n4" {} "n5" {}})
        unreachable (update reachable :metrics dissoc "n5")]
    (testing "a reachable learner is promoted without resetting its data"
      (with-redefs [cluster/membership-status (constantly reachable)
                    openraft-db/start-empty-node!
                    (fn [& _]
                      (throw (ex-info "MUST NOT reset a learner" {})))
                    client/add-learner!
                    (fn [& _]
                      (swap! calls conj :add-learner))
                    client/change-membership!
                    (fn [& _]
                      (swap! calls conj :change-membership))]
        (let [result (nemesis/invoke! (membership-nemesis)
                                      test-config
                                      {:type :info
                                       :f :grow})]
          (is (= [:add-learner :change-membership] @calls))
          (is (= after (get-in result [:value :after])))
          (is (= :installed (get-in result [:value :status]))))))

    (testing "an unreachable learner is reported without blocking"
      (reset! calls [])
      (with-redefs [cluster/membership-status (constantly unreachable)
                    client/add-learner!
                    (fn [& _]
                      (swap! calls conj :add-learner))]
        (let [result (nemesis/invoke! (membership-nemesis)
                                      test-config
                                      {:type :info
                                       :f :grow})]
          (is (= :skipped (get-in result [:value :status])))
          (is (= :learner-unreachable
                 (get-in result [:value :reason])))
          (is (empty? @calls)))))))

(deftest defers-an-indeterminate-learner-addition
  (let [before #{"n1" "n2" "n3" "n4"}
        changed? (atom false)
        status (update (stable-status before)
                       :metrics
                       assoc
                       "n5"
                       {})]
    (with-redefs [cluster/membership-status
                  (constantly status)
                  client/add-learner!
                  (fn [& _]
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= :indeterminate (get-in result [:value :status])))
        (is (= :request-result-unknown
               (get-in result [:value :reason])))
        (is (false? @changed?))))))

(deftest skips-an-add-learner-during-an-existing-membership-change
  (let [before #{"n1" "n2" "n3" "n4"}
        status (update (stable-status before) :metrics assoc "n5" {})
        changed? (atom false)
        pending (atom nil)
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database pending))]
    (with-redefs [cluster/membership-status (constantly status)
                  client/add-learner!
                  (fn [& _]
                    (throw (ex-info
                            "membership change in progress"
                            {:kind :openraft-error
                             :error {:ChangeMembershipError
                                     {:InProgress {}}}})))
                  client/change-membership!
                  (fn [& _] (reset! changed? true))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info :f :grow}))]
        (is (= :skipped (:status value)))
        (is (= :membership-change-in-progress (:reason value)))
        (is (= :add-learner (:stage value)))
        (is (= {:status :in-progress
                :reason :membership-change-in-progress
                :stage :add-learner}
               (select-keys @pending [:status :reason :stage])))
        (is (false? @changed?))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest classifies-invalid-membership-responses-after-the-attempt
  (testing "an invalid add-learner response is indeterminate"
    (let [before #{"n1" "n2" "n3" "n4"}
          status (update (stable-status before) :metrics assoc "n5" {})
          failure-state (harness/failure-state)
          changed? (atom false)
          subject (worker/wrap-nemesis failure-state
                                       (membership-nemesis))]
      (with-redefs [cluster/membership-status (constantly status)
                    client/add-learner!
                    (fn [& _]
                      (throw (ex-info "invalid response"
                                      {:kind :invalid-response})))
                    client/change-membership!
                    (fn [& _]
                      (reset! changed? true))]
        (let [setup-subject (nemesis/setup! subject test-config)
              value (:value (nemesis/invoke! setup-subject
                                             test-config
                                             {:type :info :f :grow}))]
          (is (= :indeterminate (:status value)))
          (is (= :request-result-unknown (:reason value)))
          (is (= :add-learner (:stage value)))
          (is (false? @changed?))
          (is (nil? (harness/primary-failure failure-state)))))))

  (testing "an invalid change-membership response is indeterminate"
    (let [failure-state (harness/failure-state)
          cleaned? (atom false)
          subject (worker/wrap-nemesis failure-state
                                       (membership-nemesis))]
      (with-redefs [cluster/membership-status
                    (constantly (stable-status (set nodes)))
                    random/shuffle reverse
                    client/change-membership!
                    (fn [& _]
                      (throw (ex-info "invalid response"
                                      {:kind :invalid-response})))
                    openraft-db/stop-and-wipe-node!
                    (fn [& _]
                      (reset! cleaned? true))]
        (let [setup-subject (nemesis/setup! subject test-config)
              value (:value (nemesis/invoke! setup-subject
                                             test-config
                                             {:type :info :f :shrink}))]
          (is (= :indeterminate (:status value)))
          (is (= :request-result-unknown (:reason value)))
          (is (= :change-membership (:stage value)))
          (is (false? @cleaned?))
          (is (nil? (harness/primary-failure failure-state)))))))

  (testing "an unknown request exception still records Harness failure"
    (let [before #{"n1" "n2" "n3" "n4"}
          status (update (stable-status before) :metrics assoc "n5" {})
          failure-state (harness/failure-state)
          error (RuntimeException. "request bug")
          subject (worker/wrap-nemesis failure-state
                                       (membership-nemesis))]
      (with-redefs [cluster/membership-status (constantly status)
                    client/add-learner! (fn [& _] (throw error))]
        (let [setup-subject (nemesis/setup! subject test-config)
              thrown (try
                       (nemesis/invoke! setup-subject
                                        test-config
                                        {:type :info :f :grow})
                       nil
                       (catch Exception e
                         e))]
          (is (identical? error thrown))
          (is (identical? error
                          (:throwable
                           (harness/primary-failure failure-state)))))))))

(deftest malformed-membership-error-unions-are-unexpected-sut-responses
  (doseq [[description payload expected-shape expected-count expected-names]
          [["scalar payload" "not-a-union" :not-map nil []]
           ["multiple variants"
            {:InProgress {}
             :LearnerNotFound {:node_id "n5"}}
            :map
            2
            ["InProgress" "LearnerNotFound"]]]]
    (testing (str description " from add-learner")
      (let [before #{"n1" "n2" "n3" "n4"}
            status (update (stable-status before) :metrics assoc "n5" {})
            failure-state (harness/failure-state)
            subject (worker/wrap-nemesis failure-state
                                         (membership-nemesis))]
        (with-redefs [cluster/membership-status (constantly status)
                      client/add-learner!
                      (fn [& _]
                        (throw
                         (ex-info
                          "malformed membership error"
                          {:kind :openraft-error
                           :error {:ChangeMembershipError payload}
                           :response {:body (apply str (repeat 2048 "x"))}})))]
          (let [value (:value
                       (nemesis/invoke! subject
                                        test-config
                                        {:type :info :f :grow}))]
            (is (= :indeterminate (:status value)))
            (is (= :unexpected-sut-response (:reason value)))
            (is (= :add-learner (:stage value)))
            (is (= expected-shape (:error-shape value)))
            (is (= expected-count (:error-arm-count value)))
            (is (= expected-names (:error-arm-names value)))
            (is (not (contains? value :error)))
            (is (not (contains? value :response)))
            (is (nil? (harness/primary-failure failure-state)))))))

    (testing (str description " from change-membership")
      (let [failure-state (harness/failure-state)
            cleaned? (atom false)
            subject (worker/wrap-nemesis failure-state
                                         (membership-nemesis))]
        (with-redefs [cluster/membership-status
                      (constantly (stable-status (set nodes)))
                      random/shuffle reverse
                      client/change-membership!
                      (fn [& _]
                        (throw
                         (ex-info
                          "malformed membership error"
                          {:kind :openraft-error
                           :error {:ChangeMembershipError payload}
                           :response {:body (apply str (repeat 2048 "x"))}})))
                      openraft-db/stop-and-wipe-node!
                      (fn [& _] (reset! cleaned? true))]
          (let [value (:value
                       (nemesis/invoke! subject
                                        test-config
                                        {:type :info :f :shrink}))]
            (is (= :indeterminate (:status value)))
            (is (= :unexpected-sut-response (:reason value)))
            (is (= :change-membership (:stage value)))
            (is (= expected-shape (:error-shape value)))
            (is (= expected-count (:error-arm-count value)))
            (is (= expected-names (:error-arm-names value)))
            (is (not (contains? value :error)))
            (is (not (contains? value :response)))
            (is (false? @cleaned?))
            (is (nil? (harness/primary-failure failure-state)))))))))

(deftest ordinary-grow-classifies-impossible-add-learner-errors
  (doseq [[variant payload]
          [[:LearnerNotFound {:node_id "n5"}]
           [:EmptyMembership {}]]]
    (testing (name variant)
      (let [before #{"n1" "n2" "n3" "n4"}
            status (update (stable-status before) :metrics assoc "n5" {})
            pending (atom nil)
            changed? (atom false)
            failure-state (harness/failure-state)
            subject (worker/wrap-nemesis
                     failure-state
                     (membership/->MembershipNemesis :database pending))]
        (with-redefs [cluster/membership-status (constantly status)
                      client/add-learner!
                      (fn [& _]
                        (throw
                         (ex-info
                          "unexpected add-learner response"
                          {:kind :openraft-error
                           :error {:ChangeMembershipError
                                   {variant payload}}})))
                      client/change-membership!
                      (fn [& _]
                        (reset! changed? true))]
          (let [value (:value
                       (nemesis/invoke! subject
                                        test-config
                                        {:type :info :f :grow}))]
            (is (= {:status :indeterminate
                    :reason :unexpected-sut-response
                    :stage :add-learner
                    :error-variant variant}
                   (select-keys value
                                [:status :reason :stage :error-variant])))
            (is (= {:status :indeterminate
                    :reason :unexpected-sut-response
                    :stage :add-learner
                    :error-variant variant}
                   (select-keys @pending
                                [:status :reason :stage :error-variant])))
            (is (false? @changed?))
            (is (nil? (harness/primary-failure failure-state)))))))))

(deftest defers-a-grow-when-leader-routing-is-exhausted
  (let [before #{"n1" "n2" "n3" "n4"}
        status (update (stable-status before) :metrics assoc "n5" {})
        attempts (atom [])]
    (with-redefs [cluster/membership-status (constantly status)
                  client/add-learner!
                  (fn [endpoint node-id api-addr raft-addr]
                    (swap! attempts conj
                           [endpoint node-id api-addr raft-addr])
                    (throw (ex-info "unreachable" {:kind :unreachable})))
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "MUST NOT change membership" {})))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= (mapv #(vector % "n5" "n5:21001" "n5:22001")
                     ["n1:21001" "n2:21001" "n3:21001"
                      "n4:21001" "n5:21001"])
               @attempts))
        (is (= :skipped (get-in result [:value :status])))
        (is (= :no-supported-leader
               (get-in result [:value :reason])))
        (is (= :add-learner (get-in result [:value :stage])))
        (is (= {:change :grow
                :node "n5"
                :source :non-member
                :leader "n1"
                :before before
                :target (conj before "n5")}
               (select-keys (:value result)
                            [:change :node :source :leader :before :target])))))))

(deftest confirms-an-indeterminate-grow-from-membership-state
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        statuses (atom [(update (stable-status before)
                                :metrics
                                assoc
                                "n5"
                                {})
                        (stable-status after)])
        change-attempts (atom 0)
        subject (membership-nemesis)]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  client/add-learner! (fn [& _])
                  client/change-membership!
                  (fn [& _]
                    (swap! change-attempts inc)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))]
      (let [pending (nemesis/invoke! subject
                                     test-config
                                     {:type :info
                                      :f :grow})]
        (is (= :indeterminate (get-in pending [:value :status]))))

      (let [resolved (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :shrink})]
        (is (= 1 @change-attempts))
        (is (= :skipped (get-in resolved [:value :status])))
        (is (= :pending-membership-change
               (get-in resolved [:value :reason])))
        (is (= {:change :grow
                :before before
                :after after}
               (select-keys (get-in resolved
                                    [:value :resolved-change])
                            [:change :before :after])))))))

(deftest does-not-demote-an-unresolved-indeterminate-grow
  (let [before #{"n1" "n2" "n3" "n4"}
        pending-status (update (stable-status before) :metrics assoc "n5" {})
        unresolved-status (stable-status before)
        statuses (atom [pending-status unresolved-status])
        subject (membership-nemesis)]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  client/add-learner!
                  (fn [& _]
                    (throw (ex-info "timeout" {:kind :request-timeout})))]
      (let [first-result (nemesis/invoke! subject test-config
                                          {:type :info :f :grow})
            retry-result (nemesis/invoke! subject test-config
                                          {:type :info :f :grow})]
        (is (= :indeterminate (get-in first-result [:value :status])))
        (is (= :indeterminate (get-in retry-result [:value :status])))
        (is (= :request-result-unknown
               (get-in retry-result [:value :reason])))))))

(deftest opposite-operation-preserves-an-indeterminate-pending-retry
  (let [before (set nodes)
        after (disj before "n5")
        pending (atom {:change :shrink
                       :node "n5"
                       :leader "n1"
                       :before before
                       :target after
                       :stage :change-membership})
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database pending))]
    (with-redefs [cluster/membership-status
                  (constantly (stable-status before))
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "timeout" {:kind :request-timeout})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _]
                    (throw (ex-info "MUST NOT clean up" {})))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info :f :grow}))]
        (is (= :indeterminate (:status value)))
        (is (= :request-result-unknown (:reason value)))
        (is (= :indeterminate
               (get-in value [:pending-change :status])))
        (is (= :shrink (get-in value [:pending-change :change])))
        (is (= :indeterminate (:status @pending)))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest shrinks-without-waiting-before-cleanup
  (let [calls (atom [])
        before (set nodes)
        after (disj before "n5")]
    (with-redefs [cluster/membership-status
                  (constantly (stable-status before))
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime shrink must not wait" {})))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))
                  openraft-db/stop-and-wipe-node!
                  (fn [database _test node]
                    (swap! calls conj [:stop-and-wipe database node]))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :shrink})]
        (is (= [[:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4"]]
                [:stop-and-wipe :database "n5"]]
               @calls))
        (is (= {:change :shrink
                :node "n5"
                :before before
                :after after}
               (select-keys (:value result)
                            [:change :node :before :after])))
        (is (= :installed (get-in result [:value :status])))))))

(deftest defers-cleanup-after-an-indeterminate-shrink
  (let [calls (atom [])
        before (set nodes)
        after (disj before "n5")
        statuses (atom [(stable-status before)
                        (stable-status after)])
        subject (membership-nemesis)]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (swap! statuses subvec 1)
                      status))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [& _]
                    (swap! calls conj :change-membership)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout})))
                  openraft-db/stop-and-wipe-node!
                  (fn [_database _test node]
                    (swap! calls conj [:stop-and-wipe node]))]
      (let [pending (nemesis/invoke! subject
                                     test-config
                                     {:type :info
                                      :f :shrink})]
        (is (= :indeterminate (get-in pending [:value :status])))
        (is (= [:change-membership] @calls)))

      (let [resolved (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :grow})]
        (is (= [:change-membership [:stop-and-wipe "n5"]] @calls))
        (is (= :skipped (get-in resolved [:value :status])))
        (is (= :pending-membership-change
               (get-in resolved [:value :reason])))
        (is (= {:change :shrink
                :before before
                :after after}
               (select-keys (get-in resolved
                                    [:value :resolved-change])
                            [:change :before :after])))))))

(deftest defers-a-shrink-when-leader-routing-is-exhausted
  (let [before (set nodes)
        after (disj before "n5")
        attempts (atom [])]
    (with-redefs [cluster/membership-status
                  (constantly (stable-status before))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! attempts conj [endpoint node-ids])
                    (throw (ex-info
                            "forward"
                            {:kind :openraft-error
                             :error {:ForwardToLeader
                                     {:leader_id nil
                                      :leader_node nil}}})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _]
                    (throw (ex-info "MUST NOT clean up" {})))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :shrink})]
        (is (= (mapv #(vector % ["n1" "n2" "n3" "n4"])
                     ["n1:21001" "n2:21001" "n3:21001"
                      "n4:21001" "n5:21001"])
               @attempts))
        (is (= :skipped (get-in result [:value :status])))
        (is (= :no-supported-leader
               (get-in result [:value :reason])))
        (is (= :change-membership (get-in result [:value :stage])))
        (is (= {:change :shrink
                :node "n5"
                :leader "n1"
                :before before
                :target after}
               (select-keys (:value result)
                            [:change :node :leader :before :target])))))))

(deftest skips-a-membership-change-that-is-already-in-progress
  (let [cleaned? (atom false)]
    (with-redefs [cluster/membership-status
                  (constantly (stable-status (set nodes)))
                  random/shuffle reverse
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info
                            "membership change in progress"
                            {:kind :openraft-error
                             :error {:ChangeMembershipError
                                     {:InProgress {}}}})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _]
                    (reset! cleaned? true))]
      (let [value (:value
                   (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info :f :shrink}))]
        (is (= :skipped (:status value)))
        (is (= :membership-change-in-progress (:reason value)))
        (is (= :change-membership (:stage value)))
        (is (false? @cleaned?))))))

(deftest advances-a-joint-membership-without-waiting
  (let [before #{"n1" "n2" "n3"}
        target (conj before "n4")
        joint {:leader "n1"
               :stable? false
               :effective-voter-configs [before target]}
        calls (atom [])]
    (with-redefs [cluster/membership-status (constantly joint)
                  cluster/await-stable-membership!
                  (fn [& _]
                    (throw (ex-info "runtime joint completion must not wait"
                                    {})))
                  client/change-membership!
                  (fn [endpoint node-ids]
                    (swap! calls conj
                           [:change-membership endpoint node-ids]))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :grow})]
        (is (= [[:change-membership
                 "n1:21001"
                 ["n1" "n2" "n3" "n4"]]]
               @calls))
        (is (= :skipped (get-in result [:value :status])))
        (is (= :membership-change-in-progress
               (get-in result [:value :reason])))
        (is (= :installed
               (get-in result [:value :existing-change :status])))
        (is (= target
               (get-in result [:value :existing-change :target])))))))

(deftest joint-membership-ambiguity-remains-indeterminate
  (let [before #{"n1" "n2" "n3"}
        target (conj before "n4")
        joint {:leader "n1"
               :stable? false
               :effective-voter-configs [before target]}
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis failure-state
                                     (membership-nemesis))]
    (with-redefs [cluster/membership-status (constantly joint)
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "timeout" {:kind :request-timeout})))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info :f :grow}))]
        (is (= :indeterminate (:status value)))
        (is (= :request-result-unknown (:reason value)))
        (is (= :indeterminate
               (get-in value [:existing-change :status])))
        (is (= target (get-in value [:existing-change :target])))
        (is (= [before target]
               (get-in value [:existing-change :voter-configs])))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest final-restoration-resolves-a-pending-removal
  (let [before (set nodes)
        after (disj before "n5")
        restored {:leader "n1" :voters before}
        statuses (atom [(stable-status before)
                        (stable-status after)
                        restored])
        pending (atom {:change :shrink
                       :node "n5"
                       :before before
                       :target after})
        subject (membership/->MembershipNemesis :database pending)
        calls (atom [])]
    (with-redefs-fn
      {#'membership/stable-membership!
       (fn [_test _context]
         (let [status (first @statuses)]
           (swap! statuses subvec 1)
           status))
       #'membership/change-membership-and-await!
       (fn [_test _leader-endpoint target _context]
         (swap! calls conj [:complete-removal target])
         (stable-status after))
       #'membership/grow!
       (fn [_database _test _context]
         (swap! calls conj :grow))
       #'openraft-db/stop-and-wipe-node!
       (fn [_database _test node]
         (swap! calls conj [:stop-and-wipe node]))}
      (fn []
        (let [result (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership})]
          (is (= [[:complete-removal after]
                  [:stop-and-wipe "n5"]
                  :grow]
                 @calls))
          (is (= {:leader "n1"
                  :voters before
                  :status :installed}
                 (select-keys (:value result)
                              [:leader :voters :status])))
          (is (= {:change :shrink
                  :node "n5"
                  :before before
                  :leader "n1"
                  :after after
                  :status :installed}
                 (get-in result [:value :resolved-change])))
          (is (nil? @pending)))))))

(deftest final-restoration-confirms-a-completed-grow
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        pending (atom {:change :grow
                       :node "n5"
                       :source :non-member
                       :before before
                       :target after})
        subject (membership/->MembershipNemesis :database pending)]
    (with-redefs-fn
      {#'membership/stable-membership! (fn [_test _context]
                                         (stable-status after))
       #'membership/grow! (fn [& _]
                            (throw (ex-info "MUST NOT grow again" {})))}
      (fn []
        (let [result (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership})]
          (is (= {:change :grow
                  :node "n5"
                  :source :non-member
                  :before before
                  :leader "n1"
                  :after after
                  :status :installed}
                 (get-in result [:value :resolved-change])))
          (is (= :installed (get-in result [:value :status])))
          (is (nil? @pending)))))))

(deftest final-restoration-waits-for-an-in-progress-learner-add
  (let [before #{"n1" "n2" "n3" "n4"}
        after (conj before "n5")
        initial-status (update (stable-status before)
                               :metrics
                               assoc
                               "n5"
                               {})
        learner-status (assoc initial-status
                              :learners #{"n5"}
                              :non-members #{})
        original-until! await/until!]
    (doseq [{:keys [description observed-status expected-calls]}
            [{:description "continues when the learner is already present"
              :observed-status learner-status
              :expected-calls [:start-empty
                               [:add-learner 1]
                               :membership-status
                               :change-membership
                               :await-stable]}
             {:description "retries add-learner when the node is still absent"
              :observed-status initial-status
              :expected-calls [:start-empty
                               [:add-learner 1]
                               :membership-status
                               [:add-learner 2]
                               :change-membership
                               :await-stable]}]]
      (testing description
        (let [calls (atom [])
              statuses (atom [initial-status
                              initial-status
                              (stable-status after)])
              add-attempts (atom 0)
              failure-state (harness/failure-state)
              subject (worker/wrap-nemesis
                       failure-state
                       (membership/->MembershipNemesis :database (atom nil)))]
          (with-redefs [await/until!
                        (fn [condition f opts]
                          (original-until!
                           condition
                           f
                           (assoc opts :retry-interval 0)))
                        cluster/await-committed-membership!
                        (fn [_test]
                          (let [status (first @statuses)]
                            (swap! statuses subvec 1)
                            status))
                        cluster/membership-status
                        (fn [_test]
                          (swap! calls conj :membership-status)
                          observed-status)
                        cluster/await-stable-membership!
                        (fn [_test target]
                          (swap! calls conj :await-stable)
                          (is (= after target))
                          (stable-status after))
                        openraft-db/start-empty-node!
                        (fn [& _]
                          (swap! calls conj :start-empty))
                        client/add-learner!
                        (fn [& _]
                          (let [attempt (swap! add-attempts inc)]
                            (swap! calls conj [:add-learner attempt])
                            (when (= 1 attempt)
                              (throw
                               (ex-info
                                "membership change in progress"
                                {:kind :openraft-error
                                 :error {:ChangeMembershipError
                                         {:InProgress {}}}})))))
                        client/change-membership!
                        (fn [& _]
                          (swap! calls conj :change-membership))]
            (let [value (:value
                         (nemesis/invoke! subject
                                          test-config
                                          {:type :info
                                           :f :restore-membership}))]
              (is (= :installed (:status value)))
              (is (= after (:voters value)))
              (is (= expected-calls @calls))
              (is (empty? @statuses))
              (is (nil? (harness/primary-failure failure-state))))))))))

(deftest final-restoration-bounds-an-in-progress-learner-add
  (let [before #{"n1" "n2" "n3" "n4"}
        status (update (stable-status before) :metrics assoc "n5" {})
        add-attempts (atom 0)
        changed? (atom false)
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database (atom nil)))]
    (with-redefs [membership/learner-wait-timeout-ms 0
                  cluster/await-committed-membership! (constantly status)
                  openraft-db/start-empty-node! (fn [& _])
                  client/add-learner!
                  (fn [& _]
                    (swap! add-attempts inc)
                    (throw
                     (ex-info
                      "membership change in progress"
                      {:kind :openraft-error
                       :error {:ChangeMembershipError {:InProgress {}}}})))
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :restore-membership}))]
        (is (= :skipped (:status value)))
        (is (= :membership-change-in-progress (:reason value)))
        (is (= :add-learner (:stage value)))
        (is (= :add-learner-request-ready (:condition value)))
        (is (= 1 @add-attempts))
        (is (false? @changed?))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest final-restoration-classifies-impossible-add-learner-errors
  (doseq [[variant payload]
          [[:LearnerNotFound {:node_id "n5"}]
           [:EmptyMembership {}]]]
    (testing (name variant)
      (let [before #{"n1" "n2" "n3" "n4"}
            status (update (stable-status before) :metrics assoc "n5" {})
            calls (atom [])
            failure-state (harness/failure-state)
            subject (worker/wrap-nemesis
                     failure-state
                     (membership/->MembershipNemesis :database (atom nil)))]
        (with-redefs [cluster/await-committed-membership!
                      (constantly status)
                      openraft-db/start-empty-node!
                      (fn [& _]
                        (swap! calls conj :start-empty))
                      client/add-learner!
                      (fn [endpoint node-id api-addr raft-addr]
                        (swap! calls conj :add-learner)
                        (is (= ["n1:21001"
                                "n5"
                                "n5:21001"
                                "n5:22001"]
                               [endpoint node-id api-addr raft-addr]))
                        (throw
                         (ex-info
                          "unexpected add-learner response"
                          {:kind :openraft-error
                           :error {:ChangeMembershipError
                                   {variant payload}}
                           :response {:body (apply str (repeat 2048 "x"))}})))
                      client/change-membership!
                      (fn [& _]
                        (swap! calls conj :change-membership))]
          (let [value (:value
                       (nemesis/invoke! subject
                                        test-config
                                        {:type :info
                                         :f :restore-membership}))]
            (is (= {:status :indeterminate
                    :reason :unexpected-sut-response
                    :stage :add-learner
                    :error-variant variant}
                   value))
            (is (= [:start-empty :add-learner] @calls))
            (is (not (contains? value :response)))
            (is (not (contains? value :error)))
            (is (nil? (harness/primary-failure failure-state)))))))))

(deftest final-restoration-keeps-a-prior-indeterminate-add-sticky
  (let [before #{"n1" "n2" "n3" "n4"}
        status (update (stable-status before) :metrics assoc "n5" {})
        pending (atom nil)
        add-attempts (atom 0)
        changed? (atom false)
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database pending))]
    (with-redefs [membership/learner-wait-timeout-ms 0
                  cluster/membership-status (constantly status)
                  cluster/await-committed-membership! (constantly status)
                  openraft-db/start-empty-node! (fn [& _])
                  client/add-learner!
                  (fn [& _]
                    (case (swap! add-attempts inc)
                      1 (throw (ex-info "invalid response"
                                        {:kind :invalid-response}))
                      (throw
                       (ex-info
                        "membership change in progress"
                        {:kind :openraft-error
                         :error {:ChangeMembershipError
                                 {:InProgress {}}}}))))
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [runtime-value (:value
                           (nemesis/invoke! subject
                                            test-config
                                            {:type :info :f :grow}))
            retained @pending
            final-value (:value
                         (nemesis/invoke! subject
                                          test-config
                                          {:type :info
                                           :f :restore-membership}))]
        (is (= :indeterminate (:status runtime-value)))
        (is (= :request-result-unknown (:reason runtime-value)))
        (is (= :add-learner (:stage runtime-value)))
        (is (= :indeterminate (:status retained)))
        (is (= :request-result-unknown (:reason retained)))
        (is (= :add-learner (:stage retained)))
        (is (= :indeterminate (:status final-value)))
        (is (= :request-result-unknown (:reason final-value)))
        (is (= :add-learner (:stage final-value)))
        (is (= retained @pending))
        (is (= 2 @add-attempts))
        (is (false? @changed?))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest final-restoration-preserves-an-indeterminate-request
  (let [before (set nodes)
        after (disj before "n5")
        pending-value {:change :shrink
                       :node "n5"
                       :leader "n1"
                       :before before
                       :target after
                       :stage :change-membership}
        pending (atom pending-value)
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database pending))
        cleaned? (atom false)
        timeout (condition-timeout :stable-membership)]
    (with-redefs [cluster/await-committed-membership!
                  (constantly (stable-status before))
                  cluster/await-stable-membership!
                  (fn [& _] (throw timeout))
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "invalid application response"
                                    {:kind :invalid-response})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _] (reset! cleaned? true))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :restore-membership}))]
        (is (= :indeterminate (:status value)))
        (is (= :request-result-unknown (:reason value)))
        (is (= :change-membership (:stage value)))
        (is (= pending-value @pending))
        (is (false? @cleaned?))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest final-restoration-does-not-demote-earlier-ambiguity
  (let [before (set nodes)
        after (disj before "n5")
        pending-value {:change :shrink
                       :node "n5"
                       :leader "n1"
                       :before before
                       :target after
                       :stage :change-membership
                       :status :indeterminate}
        pending (atom pending-value)
        failure-state (harness/failure-state)
        subject (worker/wrap-nemesis
                 failure-state
                 (membership/->MembershipNemesis :database pending))
        cleaned? (atom false)]
    (with-redefs [cluster/await-committed-membership!
                  (constantly (stable-status before))
                  client/change-membership!
                  (fn [& _]
                    (throw (ex-info "no leader" {:kind :unreachable})))
                  openraft-db/stop-and-wipe-node!
                  (fn [& _] (reset! cleaned? true))]
      (let [value (:value
                   (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :restore-membership}))]
        (is (= :indeterminate (:status value)))
        (is (= :request-result-unknown (:reason value)))
        (is (= :change-membership (:stage value)))
        (is (= pending-value @pending))
        (is (false? @cleaned?))
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest final-restoration-distinguishes-pre-attempt-and-harness-failures
  (testing "a modeled wait expiry before a request is skipped"
    (let [failure-state (harness/failure-state)
          subject (worker/wrap-nemesis
                   failure-state
                   (membership/->MembershipNemesis :database (atom nil)))
          timeout (condition-timeout :committed-membership)]
      (with-redefs [cluster/await-committed-membership!
                    (fn [_test] (throw timeout))]
        (let [value (:value
                     (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership}))]
          (is (= :skipped (:status value)))
          (is (= :membership-not-ready (:reason value)))
          (is (nil? (harness/primary-failure failure-state)))))))

  (testing "an unknown wait exception takes the direct Harness path"
    (let [failure-state (harness/failure-state)
          error (RuntimeException. "membership wait bug")
          subject (worker/wrap-nemesis
                   failure-state
                   (membership/->MembershipNemesis :database (atom nil)))
          thrown (with-redefs [cluster/await-committed-membership!
                               (fn [_test] (throw error))]
                   (try
                     (nemesis/invoke! subject
                                      test-config
                                      {:type :info
                                       :f :restore-membership})
                     nil
                     (catch Exception e
                       e)))]
      (is (identical? error thrown))
      (is (identical? error
                      (:throwable
                       (harness/primary-failure failure-state)))))))

(deftest preserves-the-minimum-membership
  (let [changed? (atom false)
        status (stable-status #{"n1" "n2" "n3"})]
    (with-redefs [cluster/membership-status (constantly status)
                  client/change-membership!
                  (fn [& _]
                    (reset! changed? true))]
      (let [result (nemesis/invoke! (membership-nemesis)
                                    test-config
                                    {:type :info
                                     :f :shrink})]
        (is (= (skipped :minimum-membership {}) (:value result)))
        (is (false? @changed?))))))

(deftest membership-package-requires-room-to-change
  (let [three-node-test (assoc test-config
                               :nodes ["n1" "n2" "n3"])
        error (try
                (membership/membership-package :database three-node-test)
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is (= "Membership nemesis requires more than three nodes"
           (ex-message error)))
    (is (= ["n1" "n2" "n3"]
           (:nodes (ex-data error))))))

(deftest checks-membership-coverage-and-restoration
  (let [subject (:checker (membership/membership-package
                           :database
                           test-config))
        all-voters (set nodes)
        four-voters (disj all-voters "n5")
        shrink {:type :info
                :f :grow
                :value (installed {:change :shrink
                                   :node "n5"
                                   :leader "n1"
                                   :before all-voters
                                   :after four-voters})}
        grow {:type :info
              :f :grow
              :value (installed {:change :grow
                                 :node "n5"
                                 :leader "n1"
                                 :before four-voters
                                 :after all-voters})}
        restore {:type :info
                 :f :restore-membership
                 :value (installed {:leader "n1"
                                    :voters all-voters})}
        recovery {:type :info
                  :f :await-recovery
                  :value (installed {:leader "n1"})}]
    (testing "a resolved change is attributed to its actual fault class"
      (is (:valid? (checker/check subject
                                  test-config
                                  [shrink grow restore recovery]
                                  {}))))

    (testing "an installed change without its evidence is a defect"
      (let [result (checker/check subject
                                  test-config
                                  [(assoc shrink :value (installed {}))
                                   grow
                                   restore
                                   recovery]
                                  {})]
        (is (false? (:valid? result)))
        (is (= 1 (:unrecognized-installs result)))
        (is (= [:shrink] (:missing-changes result)))))

    (testing "an indeterminate request does not count as coverage"
      (let [result (checker/check
                    subject
                    test-config
                    [(assoc shrink
                            :value (indeterminate
                                    :request-result-unknown
                                    {:change :shrink
                                     :before all-voters
                                     :target four-voters}))
                     grow
                     restore
                     recovery]
                    {})]
        (is (false? (:valid? result)))
        (is (= [:shrink] (:missing-changes result)))))

    (testing "final recovery can confirm an indeterminate change"
      (let [result (checker/check
                    subject
                    test-config
                    [(assoc shrink
                            :value (indeterminate
                                    :request-result-unknown
                                    {:change :shrink
                                     :before all-voters
                                     :target four-voters}))
                     grow
                     (assoc-in restore
                               [:value :resolved-change]
                               (:value shrink))
                     recovery]
                    {})]
        (is (:valid? result))))

    (testing "final restoration must be followed by cluster recovery"
      (let [result (checker/check subject
                                  test-config
                                  [shrink grow restore]
                                  {})]
        (is (false? (:valid? result)))
        (is (:restored? result))
        (is (false? (:recovered? result)))))

    (testing "a later failed recovery supersedes an earlier success"
      (let [result (checker/check subject
                                  test-config
                                  [shrink
                                   grow
                                   restore
                                   recovery
                                   (assoc recovery
                                          :value (indeterminate
                                                  :recovery-timeout
                                                  {}))]
                                  {})]
        (is (false? (:valid? result)))
        (is (false? (:recovered? result)))))

    (testing "the final membership must be restored"
      (let [result (checker/check subject
                                  test-config
                                  [shrink grow]
                                  {})]
        (is (false? (:valid? result)))
        (is (false? (:restored? result)))))))
